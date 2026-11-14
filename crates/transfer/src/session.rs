//! Sessions complètes d'envoi et de réception, utilisées par le `cli` et par
//! l'application. Le déroulement est signalé par des `Event`, sérialisables pour
//! l'interface.
//!
//! - `run_sender` : publie le lien, attend le destinataire, vérifie le code
//!   (3 essais), sert les fichiers, reçoit l'accusé. Reste en attente après une
//!   coupure pour permettre la reprise. Le lien expire au bout de 24 h.
//! - `run_receiver` : (attend l'heure de départ en mode Noctiluca), trouve
//!   l'émetteur, s'authentifie, reçoit et vérifie les fichiers, écrit le MHL,
//!   envoie l'accusé.
//!
//! Les deux gardent la machine éveillée pendant toute la session et peuvent la
//! mettre en veille ou l'éteindre à la fin.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use lugus_auth::{Handshake, Side, EXPORTER_LABEL};
use lugus_core::code::AccessCode;
use lugus_core::limiter::{AttemptLimiter, Verdict};
use lugus_core::link::LugusLink;
use lugus_core::protocol::{FileKind, Manifest, Message, ProtocolError, Selection};
use lugus_core::{wire, DEFAULT_LINK_TTL_SECS, PROTOCOL_VERSION};
use lugus_noctiluca::{
    channel_binding, current_path, net_err, recv_msg, remote_key, send_msg, Beacon, Connection,
    RecvStream, Role, SendStream, HANDSHAKE_MAX_LEN,
};
use lugus_store::mhl::{write_mhl, MhlEntry};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::power::{self, Awake, EndAction};
use crate::{Received, Source, DEFAULT_PARALLEL_FILES};

/// Délai maximal pour trouver l'émetteur dans la DHT et s'y connecter.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(90);
/// Délai maximal pour joindre un relais au démarrage.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(20);
/// Délai maximal pour une poignée de main complète, côté émetteur.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Délai laissé au pair pour lire nos derniers messages avant de fermer.
const LINGER: Duration = Duration::from_secs(5);
/// Intervalle des événements de progression.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

/// Nom de l'outil inscrit dans les MHL.
pub const TOOL_NAME: &str = concat!(
    "Lugus Transfer ",
    env!("CARGO_PKG_VERSION"),
    " (Domingo Olmo Martin)"
);

/// Déroulement d'une session. `path` vaut `local`, `direct`, `relay` ou `unknown`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    // --- Émetteur
    /// Lien publié, en attente du destinataire.
    Ready {
        link: String,
        code: String,
        name: String,
        files: usize,
        bytes: u64,
        /// `false` si aucun relais n'a pu être joint : connexions directes seulement.
        relay: bool,
    },
    NewCode {
        code: String,
    },
    /// Connexion entrante, avant vérification du code.
    Incoming {
        path: String,
    },
    /// Un destinataire s'est trompé de code.
    WrongCode {
        remaining: u32,
    },
    /// 3 essais ratés : il faut un nouveau code (`SenderCommand::NewCode`).
    Locked,
    Authenticated {
        path: String,
    },
    Selected {
        files: usize,
        bytes: u64,
    },
    /// Transfert coupé : le lien reste valable, en attente d'une reprise.
    Interrupted {
        message: String,
    },
    /// Fichiers livrés et accusé de réception authentique reçu.
    Delivered {
        files: u32,
        bytes: u64,
        receipt: String,
        path: String,
    },
    /// Lien expiré sans livraison.
    Expired,

    // --- Récepteur
    /// Mode Noctiluca : attente de l'heure de départ.
    Waiting {
        until: String,
        seconds: u64,
    },
    Searching,
    Found {
        seconds: f32,
        path: String,
    },
    Manifest {
        name: String,
        files: usize,
        bytes: u64,
        originals: usize,
        proxies: usize,
        sidecars: usize,
    },
    FileVerified {
        path: String,
        size: u64,
        uncached: bool,
    },
    Received {
        files: usize,
        bytes: u64,
        seconds: f64,
        mhl: String,
        folder: String,
        path: String,
    },

    // --- Commun
    Progress {
        done: u64,
        total: u64,
        path: String,
    },
    /// Action de fin sur le point d'être exécutée.
    EndAction {
        action: EndAction,
    },
    Error {
        message: String,
    },
}

/// Reçoit les événements d'une session.
pub type Emit = Arc<dyn Fn(Event) + Send + Sync>;

/// Commandes envoyées à une session d'envoi en cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderCommand {
    /// Nouveau code d'accès (le lien ne change pas).
    NewCode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendOptions {
    pub path: PathBuf,
    #[serde(default)]
    pub end_action: EndAction,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReceiveOptions {
    pub link: String,
    pub code: String,
    pub dest: PathBuf,
    #[serde(default = "default_parallel")]
    pub parallel: usize,
    /// Mode Noctiluca : heure de départ `HH:MM`.
    #[serde(default)]
    pub start_at: Option<String>,
    #[serde(default)]
    pub end_action: EndAction,
}

fn default_parallel() -> usize {
    DEFAULT_PARALLEL_FILES
}

// ---------------------------------------------------------------------------
// Émetteur
// ---------------------------------------------------------------------------

pub async fn run_sender(
    options: SendOptions,
    emit: Emit,
    mut commands: mpsc::UnboundedReceiver<SenderCommand>,
) -> Result<()> {
    let source = Source::new(&options.path)
        .with_context(|| format!("lecture de {} impossible", options.path.display()))?;
    if source.manifest().files.is_empty() {
        bail!("aucun fichier à envoyer dans {}", options.path.display());
    }
    let source = Arc::new(source);
    let _awake = Awake::hold("Lugus Transfer : envoi en cours");

    let mut access = AccessCode::generate().context("génération du code")?;
    let mut limiter = AttemptLimiter::default();

    let beacon = Beacon::bind(Role::Sender).await?;
    let relay = tokio::time::timeout(ONLINE_TIMEOUT, beacon.online())
        .await
        .is_ok();
    let manifest = source.manifest();
    emit(Event::Ready {
        link: beacon.link().to_string(),
        code: access.to_string(),
        name: manifest.name.clone(),
        files: manifest.files.len(),
        bytes: manifest.total_bytes(),
        relay,
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(DEFAULT_LINK_TTL_SECS);
    let mut commands_open = true;

    loop {
        let conn = tokio::select! {
            accepted = beacon.accept() => match accepted? {
                Some(conn) => conn,
                None => bail!("l'endpoint s'est fermé"),
            },
            command = commands.recv(), if commands_open => {
                match command {
                    Some(SenderCommand::NewCode) => {
                        access = AccessCode::generate().context("génération du code")?;
                        limiter.reset_for_new_code();
                        emit(Event::NewCode { code: access.to_string() });
                    }
                    None => commands_open = false,
                }
                continue;
            }
            _ = tokio::time::sleep_until(deadline) => {
                emit(Event::Expired);
                beacon.close().await;
                return Ok(());
            }
        };
        emit(Event::Incoming {
            path: current_path(&conn).code().to_string(),
        });

        let handshake = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            sender_handshake(&conn, &access, &mut limiter, beacon.key()),
        )
        .await;

        match handshake {
            Ok(Ok(Some(accepted))) => {
                emit(Event::Authenticated {
                    path: current_path(&conn).code().to_string(),
                });
                match deliver(&conn, source.clone(), accepted, &emit).await {
                    Ok(()) => {
                        let _ = tokio::time::timeout(LINGER, conn.closed()).await;
                        break;
                    }
                    Err(err) => {
                        emit(Event::Interrupted {
                            message: format!("{err:#}"),
                        });
                        continue;
                    }
                }
            }
            Ok(Ok(None)) => {
                let _ = tokio::time::timeout(LINGER, conn.closed()).await;
            }
            Ok(Err(_)) => {}
            Err(_) => conn.close(1u32.into(), b"delai depasse"),
        }

        if limiter.is_locked() {
            emit(Event::Locked);
            let command = tokio::select! {
                command = commands.recv(), if commands_open => command,
                _ = tokio::time::sleep_until(deadline) => None,
                else => None,
            };
            match command {
                Some(SenderCommand::NewCode) => {
                    access = AccessCode::generate().context("génération du code")?;
                    limiter.reset_for_new_code();
                    emit(Event::NewCode {
                        code: access.to_string(),
                    });
                }
                None => {
                    emit(Event::Expired);
                    beacon.close().await;
                    return Ok(());
                }
            }
        } else if limiter.failures() > 0 {
            emit(Event::WrongCode {
                remaining: limiter.remaining(),
            });
        }
    }

    beacon.close().await;
    finish_with(options.end_action, &emit);
    Ok(())
}

/// Résultat d'une poignée de main réussie : flux de contrôle et clé de session.
struct Accepted {
    send: SendStream,
    recv: RecvStream,
    session_key: [u8; 32],
}

/// Poignée de main côté émetteur. `None` si la demande a été refusée (le pair
/// a reçu un message d'erreur).
async fn sender_handshake(
    conn: &Connection,
    access: &AccessCode,
    limiter: &mut AttemptLimiter,
    own_key: [u8; 32],
) -> Result<Option<Accepted>> {
    let (mut send, mut recv) = conn.accept_bi().await.map_err(net_err)?;

    match recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? {
        Message::Hello { version } if version == PROTOCOL_VERSION => {}
        Message::Hello { .. } => {
            let error = ProtocolError::UnsupportedVersion {
                supported: PROTOCOL_VERSION,
            };
            reject(&mut send, error).await?;
            return Ok(None);
        }
        _ => {
            reject(&mut send, ProtocolError::BadMessage).await?;
            return Ok(None);
        }
    }

    let Message::Pake { payload: peer_pake } = recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? else {
        reject(&mut send, ProtocolError::BadMessage).await?;
        return Ok(None);
    };

    // L'essai est réservé ici : il compte même si le pair abandonne ensuite.
    match limiter.begin(Instant::now()) {
        Verdict::Allowed => {}
        Verdict::WaitFor(wait) => {
            let error = ProtocolError::TooSoon {
                wait_secs: wait.as_secs().max(1),
            };
            reject(&mut send, error).await?;
            return Ok(None);
        }
        Verdict::Locked => {
            reject(&mut send, ProtocolError::CodeLocked).await?;
            return Ok(None);
        }
    }

    let context = lugus_auth::Context {
        sender_id: own_key,
        receiver_id: remote_key(conn),
        channel_binding: channel_binding(conn, EXPORTER_LABEL)?,
    };
    let (handshake, own_pake) = Handshake::start(Side::Sender, &access.password(), context);
    let session = match handshake.finish(&peer_pake) {
        Ok(session) => session,
        Err(_) => {
            reject(&mut send, ProtocolError::BadMessage).await?;
            return Ok(None);
        }
    };
    send_msg(&mut send, &Message::Pake { payload: own_pake }).await?;

    let Message::Confirm { tag } = recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? else {
        reject(&mut send, ProtocolError::BadMessage).await?;
        return Ok(None);
    };
    if !session.verify_peer(&tag) {
        let error = ProtocolError::WrongCode {
            remaining: limiter.remaining(),
        };
        reject(&mut send, error).await?;
        return Ok(None);
    }

    limiter.succeed();
    send_msg(
        &mut send,
        &Message::Confirm {
            tag: session.own_tag(),
        },
    )
    .await?;
    Ok(Some(Accepted {
        send,
        recv,
        session_key: *session.key(),
    }))
}

/// Après authentification : manifeste, sélection, service des fichiers, accusé.
async fn deliver(
    conn: &Connection,
    source: Arc<Source>,
    accepted: Accepted,
    emit: &Emit,
) -> Result<()> {
    let Accepted {
        mut send,
        mut recv,
        session_key,
    } = accepted;
    let manifest = source.manifest();

    send_msg(&mut send, &Message::Manifest(manifest.clone())).await?;
    let selection = match recv_msg(&mut recv, wire::MAX_FRAME_LEN).await? {
        Message::Select(selection) => selection,
        other => bail!("sélection attendue, reçu : {other:?}"),
    };
    if selection
        .files
        .iter()
        .any(|&i| (i as usize) >= manifest.files.len())
    {
        bail!("sélection invalide reçue");
    }
    let total: u64 = selection
        .files
        .iter()
        .map(|&i| manifest.files[i as usize].size)
        .sum();
    emit(Event::Selected {
        files: selection.files.len(),
        bytes: total,
    });

    let sent = Arc::new(AtomicU64::new(0));
    let ticker = spawn_progress(emit.clone(), sent.clone(), total, conn.clone());
    let result = crate::serve(
        conn.clone(),
        source.clone(),
        recv,
        session_key,
        sent.clone(),
    )
    .await;
    ticker.abort();
    let receipt = result?;
    emit_progress(emit, &sent, total, conn);

    emit(Event::Delivered {
        files: receipt.file_count,
        bytes: receipt.total_bytes,
        receipt: lugus_store::hex(&receipt.final_hash),
        path: current_path(conn).code().to_string(),
    });
    send.finish().map_err(net_err)?;
    Ok(())
}

async fn reject(send: &mut SendStream, error: ProtocolError) -> Result<()> {
    send_msg(send, &Message::Error(error)).await?;
    send.finish().map_err(net_err)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Récepteur
// ---------------------------------------------------------------------------

pub async fn run_receiver(options: ReceiveOptions, emit: Emit) -> Result<()> {
    // Tout est vérifié en local avant d'attendre ou de contacter l'émetteur.
    let link: LugusLink = options.link.trim().parse().context("lien invalide")?;
    let access: AccessCode = options
        .code
        .parse()
        .context("code invalide (vérifié en local, aucun essai consommé)")?;
    let start_delay = match &options.start_at {
        Some(hhmm) if !hhmm.trim().is_empty() => Some((
            hhmm.trim().to_string(),
            power::delay_until(hhmm).map_err(anyhow::Error::msg)?,
        )),
        _ => None,
    };
    std::fs::create_dir_all(&options.dest)
        .with_context(|| format!("création de {} impossible", options.dest.display()))?;

    let _awake = Awake::hold("Lugus Transfer : réception en cours");

    if let Some((until, delay)) = start_delay {
        emit(Event::Waiting {
            until,
            seconds: delay.as_secs(),
        });
        tokio::time::sleep(delay).await;
    }

    emit(Event::Searching);
    let beacon = Beacon::bind(Role::Receiver).await?;
    let started = Instant::now();
    let conn = tokio::time::timeout(CONNECT_TIMEOUT, beacon.connect(&link))
        .await
        .context("émetteur introuvable dans le délai imparti")??;
    if remote_key(&conn) != *link.key() {
        bail!("la machine jointe n'est pas celle du lien");
    }
    emit(Event::Found {
        seconds: started.elapsed().as_secs_f32(),
        path: current_path(&conn).code().to_string(),
    });

    let (mut send, mut recv) = conn.open_bi().await.map_err(net_err)?;
    send_msg(
        &mut send,
        &Message::Hello {
            version: PROTOCOL_VERSION,
        },
    )
    .await?;

    let context = lugus_auth::Context {
        sender_id: *link.key(),
        receiver_id: beacon.key(),
        channel_binding: channel_binding(&conn, EXPORTER_LABEL)?,
    };
    let (handshake, own_pake) = Handshake::start(Side::Receiver, &access.password(), context);
    send_msg(&mut send, &Message::Pake { payload: own_pake }).await?;

    let session = match recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? {
        Message::Pake { payload } => handshake.finish(&payload)?,
        Message::Error(error) => bail!(describe(&error)),
        other => bail!("réponse inattendue : {other:?}"),
    };
    send_msg(
        &mut send,
        &Message::Confirm {
            tag: session.own_tag(),
        },
    )
    .await?;

    match recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? {
        Message::Confirm { tag } if session.verify_peer(&tag) => {}
        Message::Confirm { .. } => bail!("l'émetteur n'a pas prouvé qu'il connaît le code"),
        Message::Error(error) => bail!(describe(&error)),
        other => bail!("réponse inattendue : {other:?}"),
    }

    let manifest = match recv_msg(&mut recv, wire::MAX_FRAME_LEN).await? {
        Message::Manifest(manifest) => manifest,
        other => bail!("manifeste attendu, reçu : {other:?}"),
    };
    if let Some(bad) = manifest
        .files
        .iter()
        .find(|f| !lugus_store::is_safe_relative_path(&f.path))
    {
        bail!("chemin dangereux dans le manifeste : {}", bad.path);
    }
    let layout = lugus_store::layout(&options.dest, &manifest)?;
    emit(manifest_event(&manifest));

    // Pour l'instant, tout est choisi, proxys d'abord.
    let selection = Selection {
        files: (0..manifest.files.len() as u32).collect(),
        proxies_first: true,
    };
    let order = crate::order(&manifest, &selection);
    let total: u64 = order.iter().map(|&i| manifest.files[i as usize].size).sum();
    send_msg(&mut send, &Message::Select(selection)).await?;

    let transfer_started = Instant::now();
    let mhl_started = chrono::Local::now();
    let received = Arc::new(AtomicU64::new(0));
    let ticker = spawn_progress(emit.clone(), received.clone(), total, conn.clone());
    let on_done: crate::OnFileDone = {
        let emit = emit.clone();
        let manifest = Arc::new(manifest.clone());
        Arc::new(move |done: &Received| {
            let size = manifest
                .files
                .get(done.index as usize)
                .map(|f| f.size)
                .unwrap_or(0);
            emit(Event::FileVerified {
                path: done.path.clone(),
                size,
                uncached: done.verified.uncached,
            });
        })
    };
    let result = crate::fetch(
        &conn,
        Arc::new(manifest.clone()),
        order,
        layout.base.clone(),
        options.parallel,
        received.clone(),
        on_done,
    )
    .await;
    ticker.abort();
    let results = result?;
    emit_progress(&emit, &received, total, &conn);

    // MHL 1.1 (xxHash64) dans le dossier reçu.
    let finished = chrono::Local::now();
    let entries: Vec<MhlEntry> = results
        .iter()
        .map(|done| {
            let entry = &manifest.files[done.index as usize];
            MhlEntry {
                path: done.path.clone(),
                size: entry.size,
                modified_unix: entry.modified_unix,
                xxh64: done.verified.xxh64,
            }
        })
        .collect();
    let mhl_name = lugus_store::mhl_file_name(&manifest.name, &finished);
    let mhl_path = write_mhl(
        &layout.base,
        &mhl_name,
        &entries,
        mhl_started,
        finished,
        TOOL_NAME,
    )
    .context("écriture du MHL")?;

    // Accusé de réception : empreinte BLAKE3 de la liste des hash.
    let blake_list: Vec<(String, lugus_core::protocol::Hash)> = results
        .iter()
        .map(|done| (done.path.clone(), done.verified.blake3))
        .collect();
    let (_, list_digest) = lugus_store::hash_list(&blake_list);
    let receipt = crate::make_receipt(results.len() as u32, total, list_digest, session.key());
    send_msg(&mut send, &Message::Receipt(receipt)).await?;
    send.finish().map_err(net_err)?;

    emit(Event::Received {
        files: results.len(),
        bytes: total,
        seconds: transfer_started.elapsed().as_secs_f64(),
        mhl: mhl_path.display().to_string(),
        folder: layout.base.display().to_string(),
        path: current_path(&conn).code().to_string(),
    });

    // Attend que l'émetteur ait lu l'accusé et fermé son côté.
    let _ = tokio::time::timeout(LINGER, recv.read_to_end(0)).await;
    conn.close(0u32.into(), b"fin");
    beacon.close().await;
    finish_with(options.end_action, &emit);
    Ok(())
}

fn manifest_event(manifest: &Manifest) -> Event {
    let count = |kind: FileKind| manifest.files.iter().filter(|f| f.kind == kind).count();
    Event::Manifest {
        name: manifest.name.clone(),
        files: manifest.files.len(),
        bytes: manifest.total_bytes(),
        originals: count(FileKind::Original),
        proxies: count(FileKind::Proxy),
        sidecars: count(FileKind::Sidecar),
    }
}

/// Message d'erreur pour un refus de l'émetteur.
pub fn describe(error: &ProtocolError) -> String {
    match error {
        ProtocolError::WrongCode { remaining: 0 } => {
            "code incorrect, plus aucun essai : demandez un nouveau code à l'expéditeur".to_string()
        }
        ProtocolError::WrongCode { remaining } => {
            format!("code incorrect, {remaining} essai(s) restant(s)")
        }
        ProtocolError::TooSoon { wait_secs } => {
            format!("trop tôt après un essai raté, réessayez dans {wait_secs} s")
        }
        ProtocolError::CodeLocked => {
            "ce code est bloqué : demandez un nouveau code à l'expéditeur".to_string()
        }
        ProtocolError::UnsupportedVersion { supported } => {
            format!("version incompatible, l'expéditeur utilise lugus/{supported}")
        }
        ProtocolError::LinkExpired => "ce lien a expiré".to_string(),
        ProtocolError::Busy => "l'expéditeur est occupé, réessayez plus tard".to_string(),
        ProtocolError::BadMessage => "l'expéditeur a refusé un message mal formé".to_string(),
        ProtocolError::SourceChanged => {
            "un fichier a changé chez l'expéditeur depuis la création du lien".to_string()
        }
        ProtocolError::FileUnavailable => "un fichier est illisible chez l'expéditeur".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Commun
// ---------------------------------------------------------------------------

fn spawn_progress(
    emit: Emit,
    counter: Arc<AtomicU64>,
    total: u64,
    conn: Connection,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(PROGRESS_INTERVAL);
        loop {
            interval.tick().await;
            emit_progress(&emit, &counter, total, &conn);
        }
    })
}

fn emit_progress(emit: &Emit, counter: &AtomicU64, total: u64, conn: &Connection) {
    emit(Event::Progress {
        done: counter.load(Ordering::Relaxed),
        total,
        path: current_path(conn).code().to_string(),
    });
}

fn finish_with(action: EndAction, emit: &Emit) {
    if action == EndAction::Nothing {
        return;
    }
    emit(Event::EndAction { action });
    if let Err(err) = power::perform(action) {
        emit(Event::Error {
            message: format!("action de fin impossible : {err}"),
        });
    }
}
