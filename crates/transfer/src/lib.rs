//! Transfert des fichiers, une fois la poignée de main réussie.
//!
//! - **Émetteur** (`serve`) : accepte un flux par fichier demandé, lit le
//!   fichier par blocs de 4 Mio, calcule le BLAKE3 de chaque bloc et du fichier
//!   entier, et les envoie avec les données. Plusieurs fichiers sont servis en
//!   parallèle. S'arrête à la réception de l'accusé.
//! - **Récepteur** (`fetch`) : plusieurs fichiers à la fois, écriture vérifiée
//!   et reprise par `lugus_store::incoming`, relecture finale du disque.
//!
//! L'intégrité en route est déjà garantie par QUIC (chiffrement authentifié).
//! Les hash servent à vérifier ce qui est réellement écrit sur le disque et à
//! reprendre un transfert interrompu sans doute possible.
//!
//! Les sessions complètes (poignée de main, événements, anti-veille, mode
//! Noctiluca) sont dans le module `session`.

pub mod power;
pub mod session;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use lugus_core::protocol::{
    FileKind, Hash, Manifest, Message, ProtocolError, Receipt, Selection, BLOCK_SIZE,
};
use lugus_noctiluca::{
    net_err, recv_msg, send_msg, Connection, RecvStream, SendStream, HANDSHAKE_MAX_LEN,
};
use lugus_store::incoming::{hash_file, Incoming, Opened, Verified};
use tokio::io::AsyncReadExt;
use tokio::task::{spawn_blocking, JoinSet};

/// Nombre de fichiers reçus en parallèle par défaut.
pub const DEFAULT_PARALLEL_FILES: usize = 4;

// ---------------------------------------------------------------------------
// Émetteur
// ---------------------------------------------------------------------------

/// Contenu proposé par l'émetteur : le manifeste et le dossier où le lire.
pub struct Source {
    root: PathBuf,
    manifest: Manifest,
}

impl Source {
    pub fn new(path: &Path) -> std::io::Result<Self> {
        Self::with_selection(path, None)
    }

    /// Comme `new`, en ne proposant que les chemins `only` (relatifs au dossier,
    /// séparateur `/`) s'ils sont donnés. Seuls les éléments concernés sont
    /// relus, pas tout le dossier.
    pub fn with_selection(path: &Path, only: Option<&[String]>) -> std::io::Result<Self> {
        let manifest = match only {
            Some(only) => {
                let parts = lugus_store::scan_parts(path, &lugus_store::top_levels(only))?;
                lugus_store::keep_only(parts, only)?
            }
            None => lugus_store::scan(path)?,
        };
        Ok(Self {
            root: lugus_store::source_root(path)?,
            manifest,
        })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
}

/// Avancement annoncé par le récepteur (messages `Status`).
#[derive(Debug, Default)]
pub struct PeerProgress {
    pub received: AtomicU64,
    pub verified: AtomicU64,
}

/// Sert les fichiers demandés jusqu'à la réception de l'accusé, qui est
/// vérifié avec la clé de session. `sent` compte les octets envoyés, `peer`
/// reçoit l'avancement annoncé par le récepteur.
pub async fn serve(
    conn: Connection,
    source: Arc<Source>,
    mut control: RecvStream,
    session_key: [u8; 32],
    sent: Arc<AtomicU64>,
    peer: Arc<PeerProgress>,
) -> Result<Receipt> {
    // Lecture du flux de contrôle dans sa propre tâche : une lecture interrompue
    // par `select!` perdrait des octets. Les `Status` mettent `peer` à jour ;
    // le premier autre message (l'accusé, normalement) termine la tâche.
    let mut receipt_task = tokio::spawn(async move {
        loop {
            match recv_msg(&mut control, HANDSHAKE_MAX_LEN).await? {
                Message::Status { received, verified } => {
                    peer.received.store(received, Ordering::Relaxed);
                    peer.verified.store(verified, Ordering::Relaxed);
                }
                other => return Ok::<Message, anyhow::Error>(other),
            }
        }
    });
    let mut files = JoinSet::new();

    let message = loop {
        tokio::select! {
            result = &mut receipt_task => {
                break result.map_err(|e| anyhow!("lecture de l'accusé interrompue : {e}"))??;
            }
            stream = conn.accept_bi() => {
                let (send, recv) = stream.map_err(net_err).context("connexion perdue")?;
                let source = source.clone();
                let sent = sent.clone();
                files.spawn(async move {
                    // Les erreurs sont signalées au récepteur sur le flux du fichier.
                    let _ = serve_file(send, recv, source, sent).await;
                });
            }
        }
    };

    let receipt = match message {
        Message::Receipt(receipt) => receipt,
        other => bail!("accusé de réception attendu, reçu : {other:?}"),
    };
    if !verify_receipt(&receipt, &session_key) {
        bail!("accusé de réception non authentique");
    }
    files.abort_all();
    Ok(receipt)
}

async fn serve_file(
    mut send: SendStream,
    mut recv: RecvStream,
    source: Arc<Source>,
    sent: Arc<AtomicU64>,
) -> Result<()> {
    let Message::Request { file, offset } = recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? else {
        return refuse(&mut send, ProtocolError::BadMessage).await;
    };
    let Some(entry) = source.manifest.files.get(file as usize) else {
        return refuse(&mut send, ProtocolError::BadMessage).await;
    };
    if offset > entry.size || (offset % BLOCK_SIZE != 0 && offset != entry.size) {
        return refuse(&mut send, ProtocolError::BadMessage).await;
    }

    let path = lugus_store::source_path(&source.root, entry);
    let Ok(mut reader) = tokio::fs::File::open(&path).await else {
        return refuse(&mut send, ProtocolError::FileUnavailable).await;
    };
    match reader.metadata().await {
        Ok(metadata) if metadata.len() == entry.size => {}
        _ => return refuse(&mut send, ProtocolError::SourceChanged).await,
    }

    // Le fichier est lu en entier pour calculer son hash complet ; seuls les
    // blocs à partir de `offset` sont envoyés.
    let mut full = blake3::Hasher::new();
    let mut buffer = vec![0u8; BLOCK_SIZE as usize];
    let mut position = 0u64;
    while position < entry.size {
        let len = BLOCK_SIZE.min(entry.size - position) as usize;
        if reader.read_exact(&mut buffer[..len]).await.is_err() {
            return refuse(&mut send, ProtocolError::SourceChanged).await;
        }
        let block = &buffer[..len];
        full.update(block);
        if position >= offset {
            let header = Message::Block {
                offset: position,
                len: len as u32,
                hash: *blake3::hash(block).as_bytes(),
            };
            send_msg(&mut send, &header).await?;
            send.write_all(block).await.map_err(net_err)?;
            sent.fetch_add(len as u64, Ordering::Relaxed);
        }
        position += len as u64;
    }

    let done = Message::FileDone {
        full_hash: Some(*full.finalize().as_bytes()),
    };
    send_msg(&mut send, &done).await?;
    send.finish().map_err(net_err)?;
    Ok(())
}

async fn refuse(send: &mut SendStream, error: ProtocolError) -> Result<()> {
    send_msg(send, &Message::Error(error)).await?;
    send.finish().map_err(net_err)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Récepteur
// ---------------------------------------------------------------------------

/// Fichier reçu et vérifié : indice et chemin relatif dans le manifeste, et
/// résultat de la relecture du disque.
#[derive(Debug, Clone)]
pub struct Received {
    pub index: u32,
    pub path: String,
    pub verified: Verified,
}

/// Appelé à chaque fichier vérifié.
pub type OnFileDone = Arc<dyn Fn(&Received) + Send + Sync>;

/// Ordre de demande des fichiers : la sélection, avec les proxys puis les
/// fichiers annexes avant les originaux si demandé.
pub fn order(manifest: &Manifest, selection: &Selection) -> Vec<u32> {
    let mut order: Vec<u32> = selection
        .files
        .iter()
        .copied()
        .filter(|&i| (i as usize) < manifest.files.len())
        .collect();
    if selection.proxies_first {
        let rank = |i: &u32| match manifest.files[*i as usize].kind {
            FileKind::Proxy => 0,
            FileKind::Sidecar => 1,
            FileKind::Original => 2,
        };
        order.sort_by_key(rank);
    }
    order
}

/// Octets déjà présents à destination pour chaque fichier de `order`, lus sans
/// rien modifier (indice dans le manifeste, octets). Leur somme est le point de
/// départ du compteur `received` de `fetch`.
pub fn already_received(manifest: &Manifest, order: &[u32], base: &Path) -> Vec<(u32, u64)> {
    order
        .iter()
        .filter_map(|&i| {
            let entry = manifest.files.get(i as usize)?;
            Some((i, lugus_store::incoming::resume_point(base, entry)))
        })
        .collect()
}

/// Reçoit les fichiers dans `base`, `parallel` à la fois. `received` doit déjà
/// contenir la somme de `already` (voir `already_received`) : chaque fichier
/// corrige ensuite l'écart avec ce qu'il trouve réellement sur le disque, pour
/// que le compteur ne recule jamais à la reprise.
#[allow(clippy::too_many_arguments)]
pub async fn fetch(
    conn: &Connection,
    manifest: Arc<Manifest>,
    order: Vec<u32>,
    base: PathBuf,
    parallel: usize,
    already: Vec<(u32, u64)>,
    received: Arc<AtomicU64>,
    verified: Arc<AtomicU64>,
    on_done: OnFileDone,
) -> Result<Vec<Received>> {
    let already: Arc<HashMap<u32, u64>> = Arc::new(already.into_iter().collect());
    let queue = Arc::new(Mutex::new(VecDeque::from(order)));
    let results = Arc::new(Mutex::new(Vec::new()));
    let mut workers = JoinSet::new();

    for _ in 0..parallel.max(1) {
        let conn = conn.clone();
        let manifest = manifest.clone();
        let queue = queue.clone();
        let results = results.clone();
        let base = base.clone();
        let received = received.clone();
        let verified = verified.clone();
        let on_done = on_done.clone();
        let already = already.clone();
        workers.spawn(async move {
            while let Some(index) = next_job(&queue) {
                let counted = already.get(&index).copied().unwrap_or(0);
                let done = fetch_file(
                    &conn, &manifest, index, &base, counted, &received, &verified,
                )
                .await?;
                on_done(&done);
                results.lock().expect("verrou empoisonné").push(done);
            }
            Ok::<(), anyhow::Error>(())
        });
    }

    while let Some(result) = workers.join_next().await {
        // En cas d'erreur, le retour anticipé arrête les autres fichiers.
        result.map_err(|e| anyhow!("tâche de réception interrompue : {e}"))??;
    }

    let results = std::mem::take(&mut *results.lock().expect("verrou empoisonné"));
    Ok(results)
}

fn next_job(queue: &Mutex<VecDeque<u32>>) -> Option<u32> {
    queue.lock().expect("verrou empoisonné").pop_front()
}

/// Ajoute `actual` au compteur et retire `counted` (déjà compté au départ),
/// dans cet ordre pour ne jamais passer sous zéro.
fn settle(received: &AtomicU64, actual: u64, counted: u64) {
    received.fetch_add(actual, Ordering::Relaxed);
    let current = received.load(Ordering::Relaxed);
    received.fetch_sub(counted.min(current), Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Fichiers occupés
// ---------------------------------------------------------------------------

/// Fichiers de destination en cours d'écriture ou de vérification dans ce
/// processus. Après un arrêt, une vérification lancée par l'ancienne session
/// peut encore tourner : la nouvelle session attend qu'elle se termine avant de
/// rouvrir le même fichier.
static BUSY: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Réservation d'un fichier de destination, libérée quand elle est détruite.
/// Elle accompagne chaque travail bloquant sur ce fichier, pour n'être libérée
/// qu'à la fin réelle de l'écriture ou de la vérification.
pub struct Busy(PathBuf);

impl Busy {
    pub fn try_take(path: &Path) -> Option<Busy> {
        let mut busy = BUSY.lock().expect("verrou empoisonné");
        if busy.iter().any(|p| p == path) {
            return None;
        }
        busy.push(path.to_path_buf());
        Some(Busy(path.to_path_buf()))
    }

    pub async fn take(path: &Path) -> Busy {
        loop {
            if let Some(busy) = Busy::try_take(path) {
                return busy;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        if let Ok(mut busy) = BUSY.lock() {
            busy.retain(|p| p != &self.0);
        }
    }
}

fn destination(base: &Path, entry_path: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for part in entry_path.split('/') {
        path.push(part);
    }
    path
}

async fn fetch_file(
    conn: &Connection,
    manifest: &Manifest,
    index: u32,
    base: &Path,
    counted: u64,
    received: &AtomicU64,
    verified: &Arc<AtomicU64>,
) -> Result<Received> {
    let entry = manifest
        .files
        .get(index as usize)
        .context("indice de fichier invalide")?
        .clone();

    let busy = Busy::take(&destination(base, &entry.path)).await;
    let (opened, busy) = {
        let base = base.to_path_buf();
        let entry = entry.clone();
        spawn_blocking(move || (Incoming::open(&base, &entry), busy)).await?
    };
    let opened = opened?;

    let (mut send, mut recv) = conn.open_bi().await.map_err(net_err)?;

    match opened {
        Opened::Present(path) => {
            settle(received, entry.size, counted);
            // Déjà là : on demande seulement le hash de la source pour comparer.
            let request = Message::Request {
                file: index,
                offset: entry.size,
            };
            send_msg(&mut send, &request).await?;
            send.finish().map_err(net_err)?;
            let source_hash = expect_done(&mut recv, &entry.path).await?;
            let reread = verified.clone();
            let local = spawn_blocking(move || {
                let _busy = busy;
                hash_file(&path, Some(&reread))
            })
            .await??;
            if source_hash != Some(local.blake3) {
                bail!(
                    "{} existe déjà à destination mais diffère de la source",
                    entry.path
                );
            }
            Ok(Received {
                index,
                path: entry.path,
                verified: local,
            })
        }
        Opened::Partial(mut incoming) => {
            let mut busy = busy;
            let offset = incoming.offset();
            settle(received, offset, counted);
            send_msg(
                &mut send,
                &Message::Request {
                    file: index,
                    offset,
                },
            )
            .await?;
            send.finish().map_err(net_err)?;

            loop {
                match recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? {
                    Message::Block { offset, len, hash } => {
                        if u64::from(len) > BLOCK_SIZE {
                            bail!("{} : bloc trop grand", entry.path);
                        }
                        let mut data = vec![0u8; len as usize];
                        recv.read_exact(&mut data).await.map_err(net_err)?;
                        let (back, held, result) = spawn_blocking(move || {
                            let result = incoming.write_block(offset, &data, &hash);
                            (incoming, busy, result)
                        })
                        .await?;
                        incoming = back;
                        busy = held;
                        result.with_context(|| format!("écriture de {}", entry.path))?;
                        received.fetch_add(u64::from(len), Ordering::Relaxed);
                    }
                    Message::FileDone { full_hash } => {
                        let reread = verified.clone();
                        let result = spawn_blocking(move || {
                            let _busy = busy;
                            incoming.finish(full_hash, Some(&reread))
                        })
                        .await?
                        .with_context(|| format!("vérification de {}", entry.path))?;
                        return Ok(Received {
                            index,
                            path: entry.path,
                            verified: result,
                        });
                    }
                    Message::Error(error) => {
                        bail!("{} : refusé par l'émetteur ({error:?})", entry.path)
                    }
                    other => bail!("{} : message inattendu {other:?}", entry.path),
                }
            }
        }
    }
}

async fn expect_done(recv: &mut RecvStream, path: &str) -> Result<Option<Hash>> {
    match recv_msg(recv, HANDSHAKE_MAX_LEN).await? {
        Message::FileDone { full_hash } => Ok(full_hash),
        Message::Error(error) => bail!("{path} : refusé par l'émetteur ({error:?})"),
        other => bail!("{path} : message inattendu {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Accusé de réception
// ---------------------------------------------------------------------------

/// Construit l'accusé : `final_hash` est le BLAKE3 de la liste des hash écrite
/// par le récepteur, authentifié avec la clé de session.
pub fn make_receipt(
    file_count: u32,
    total_bytes: u64,
    final_hash: Hash,
    session_key: &[u8; 32],
) -> Receipt {
    Receipt {
        file_count,
        total_bytes,
        final_hash,
        mac: *blake3::keyed_hash(session_key, &final_hash).as_bytes(),
    }
}

/// Vérifie qu'un accusé vient bien du destinataire authentifié.
pub fn verify_receipt(receipt: &Receipt, session_key: &[u8; 32]) -> bool {
    blake3::keyed_hash(session_key, &receipt.final_hash) == blake3::Hash::from_bytes(receipt.mac)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lugus_core::protocol::FileEntry;

    fn file(path: &str, kind: FileKind) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            size: 1,
            modified_unix: None,
            kind,
            root_hash: None,
        }
    }

    #[test]
    fn proxies_then_sidecars_then_originals() {
        let manifest = Manifest {
            name: "J04".to_string(),
            files: vec![
                file("A001/clip.mov", FileKind::Original),
                file("A001/clip.xml", FileKind::Sidecar),
                file("Proxy/clip.mov", FileKind::Proxy),
            ],
        };
        let selection = Selection {
            files: vec![0, 1, 2, 99],
            proxies_first: true,
        };
        assert_eq!(order(&manifest, &selection), vec![2, 1, 0]);

        let as_is = Selection {
            files: vec![0, 2],
            proxies_first: false,
        };
        assert_eq!(order(&manifest, &as_is), vec![0, 2]);
    }

    #[test]
    fn busy_file_is_reserved_until_released() {
        let path = std::env::temp_dir().join("lugus-busy-test/A001/clip.mov");
        let first = Busy::try_take(&path).expect("libre au départ");
        assert!(Busy::try_take(&path).is_none(), "déjà réservé");
        let other = Busy::try_take(&path.with_file_name("clip2.mov")).expect("autre fichier libre");
        drop(first);
        assert!(Busy::try_take(&path).is_some(), "libéré");
        drop(other);
    }

    #[test]
    fn counter_never_goes_below_zero() {
        let received = AtomicU64::new(0);
        settle(&received, 0, 10);
        assert_eq!(received.load(Ordering::Relaxed), 0);
        received.store(100, Ordering::Relaxed);
        settle(&received, 40, 30);
        assert_eq!(received.load(Ordering::Relaxed), 110);
    }

    #[test]
    fn receipt_roundtrip() {
        let key = [5u8; 32];
        let receipt = make_receipt(3, 1000, [7u8; 32], &key);
        assert!(verify_receipt(&receipt, &key));
        assert!(!verify_receipt(&receipt, &[6u8; 32]));

        let mut forged = receipt.clone();
        forged.final_hash = [8u8; 32];
        assert!(!verify_receipt(&forged, &key));
    }
}
