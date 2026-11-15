//! Réception d'un fichier : écriture par blocs, reprise, vérification finale.
//!
//! Pendant la réception, le fichier s'appelle `<nom>.lugus-part`. À côté, un
//! journal `<nom>.lugus-journal` enregistre le hash de chaque bloc écrit :
//!
//! - en-tête de 32 octets : `LUGJ`, version, taille attendue, date de
//!   modification et taille de bloc du fichier source ;
//! - puis 32 octets par bloc reçu (hash BLAKE3 envoyé par l'émetteur).
//!
//! Un bloc est d'abord écrit dans le fichier partiel, puis son hash est ajouté
//! au journal. Après une coupure, on repart du dernier bloc journalisé.
//!
//! À la fin, le fichier est **relu depuis le disque, sans le cache du système**
//! (module `rawio`, repris de Bandua) : chaque bloc est comparé à son hash et le
//! hash du fichier entier à celui de l'émetteur. Le xxHash64 du MHL est calculé
//! pendant cette même relecture. Le fichier est ensuite daté comme la source et
//! renommé. Si la vérification échoue, le journal est effacé : une nouvelle
//! tentative reprendra le fichier depuis le début.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use lugus_core::protocol::{FileEntry, Hash, BLOCK_SIZE};
use xxhash_rust::xxh64::Xxh64;

use crate::is_safe_relative_path;
use crate::rawio::{self, AlignedBuf, ReadError};

/// Résultat de la vérification d'un fichier sur le disque de destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verified {
    /// BLAKE3 du fichier entier (identique à celui de la source).
    pub blake3: Hash,
    /// xxHash64 du fichier, pour le MHL.
    pub xxh64: u64,
    /// `true` si la relecture a réellement contourné le cache du système.
    pub uncached: bool,
}

const PART_SUFFIX: &str = ".lugus-part";
const JOURNAL_SUFFIX: &str = ".lugus-journal";
const JOURNAL_MAGIC: &[u8; 4] = b"LUGJ";
const JOURNAL_VERSION: u8 = 1;
const JOURNAL_HEADER_LEN: u64 = 32;
const HASH_LEN: u64 = 32;
/// Valeur stockée quand la date de modification est inconnue.
const NO_MTIME: i64 = i64::MIN;

/// Résultat de l'ouverture d'un fichier à recevoir.
pub enum Opened {
    /// Le fichier final existe déjà avec la bonne taille : rien à télécharger,
    /// il faudra seulement le hasher.
    Present(PathBuf),
    /// Fichier à recevoir, éventuellement en reprise.
    Partial(Incoming),
}

/// Fichier en cours de réception.
pub struct Incoming {
    final_path: PathBuf,
    part_path: PathBuf,
    journal_path: PathBuf,
    part: File,
    journal: File,
    size: u64,
    modified_unix: Option<i64>,
    hashes: Vec<Hash>,
}

impl Incoming {
    /// Prépare la réception de `entry` sous le dossier `base`.
    pub fn open(base: &Path, entry: &FileEntry) -> io::Result<Opened> {
        if !is_safe_relative_path(&entry.path) {
            return Err(invalid(format!("chemin refusé : {}", entry.path)));
        }
        let mut final_path = base.to_path_buf();
        for part in entry.path.split('/') {
            final_path.push(part);
        }
        let part_path = with_suffix(&final_path, PART_SUFFIX);
        let journal_path = with_suffix(&final_path, JOURNAL_SUFFIX);

        if let Ok(metadata) = fs::metadata(&final_path) {
            if metadata.is_file() && metadata.len() == entry.size {
                // Coupure possible entre le renommage et la suppression du journal.
                let _ = fs::remove_file(&journal_path);
                return Ok(Opened::Present(final_path));
            }
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "{} existe déjà avec une taille différente",
                    final_path.display()
                ),
            ));
        }

        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let header = journal_header(entry.size, entry.modified_unix);
        let mut hashes = read_journal(&journal_path, &header).unwrap_or_default();

        let mut journal = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&journal_path)?;
        let part = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&part_path)?;
        // macOS : les pages écrites ne restent pas en cache, la relecture lira le disque.
        rawio::prepare_write(&part);

        // Garde uniquement les blocs journalisés dont les données sont sur le disque.
        let part_len = part.metadata()?.len();
        while !hashes.is_empty() && covered(hashes.len(), entry.size) > part_len {
            hashes.pop();
        }
        if hashes.len() as u64 > entry.size.div_ceil(BLOCK_SIZE) {
            hashes.clear();
        }

        // Réécrit l'en-tête et tronque le journal et le fichier partiel au point de reprise.
        journal.set_len(0)?;
        journal.seek(SeekFrom::Start(0))?;
        journal.write_all(&header)?;
        for hash in &hashes {
            journal.write_all(hash)?;
        }
        journal.flush()?;
        part.set_len(covered(hashes.len(), entry.size))?;

        Ok(Opened::Partial(Incoming {
            final_path,
            part_path,
            journal_path,
            part,
            journal,
            size: entry.size,
            modified_unix: entry.modified_unix,
            hashes,
        }))
    }

    /// Position à partir de laquelle demander la suite du fichier.
    pub fn offset(&self) -> u64 {
        covered(self.hashes.len(), self.size)
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn is_complete(&self) -> bool {
        self.offset() == self.size
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    /// Écrit le bloc suivant. `hash` est celui calculé par l'émetteur.
    pub fn write_block(&mut self, offset: u64, data: &[u8], hash: &Hash) -> io::Result<()> {
        if offset != self.offset() {
            return Err(invalid(format!(
                "bloc inattendu à {offset}, attendu à {}",
                self.offset()
            )));
        }
        let expected = BLOCK_SIZE.min(self.size - offset);
        if data.len() as u64 != expected {
            return Err(invalid(format!(
                "bloc de {} octets, attendu {expected}",
                data.len()
            )));
        }
        if blake3::hash(data) != blake3::Hash::from_bytes(*hash) {
            return Err(invalid(format!("bloc altéré à {offset}")));
        }

        self.part.seek(SeekFrom::Start(offset))?;
        self.part.write_all(data)?;
        self.journal.seek(SeekFrom::End(0))?;
        self.journal.write_all(hash)?;
        self.hashes.push(*hash);
        Ok(())
    }

    /// Relit le fichier depuis le disque (sans cache), le vérifie, le date et
    /// le renomme. En cas d'échec, le journal est effacé pour tout reprendre.
    /// `progress` compte les octets relus, pour l'affichage.
    pub fn finish(
        mut self,
        sender_full_hash: Option<Hash>,
        progress: Option<&AtomicU64>,
    ) -> io::Result<Verified> {
        if !self.is_complete() {
            return Err(invalid("fichier incomplet".to_string()));
        }
        self.part.flush()?;
        self.part.sync_all()?;

        let result = verify(&self.part_path, self.size, Some(&self.hashes), progress);
        let verified = match result {
            Ok(verified) => verified,
            Err(err) => {
                self.forget_progress();
                return Err(with_path(err, &self.final_path));
            }
        };
        if let Some(expected) = sender_full_hash {
            if expected != verified.blake3 {
                self.forget_progress();
                return Err(invalid(format!(
                    "{} : hash du fichier différent de la source",
                    self.final_path.display()
                )));
            }
        }

        if let Some(secs) = self.modified_unix.filter(|s| *s >= 0) {
            let time = UNIX_EPOCH + Duration::from_secs(secs as u64);
            self.part.set_modified(time)?;
        }
        let Incoming {
            final_path,
            part_path,
            journal_path,
            part,
            journal,
            ..
        } = self;
        drop(part);
        drop(journal);
        fs::rename(&part_path, &final_path)?;
        let _ = fs::remove_file(&journal_path);
        Ok(verified)
    }

    /// Efface le journal : la prochaine ouverture reprendra le fichier à zéro.
    fn forget_progress(&mut self) {
        let _ = self.journal.set_len(0);
        let _ = fs::remove_file(&self.journal_path);
    }
}

/// Hash d'un fichier déjà présent sur le disque (relu sans cache).
pub fn hash_file(path: &Path, progress: Option<&AtomicU64>) -> io::Result<Verified> {
    let size = fs::metadata(path)?.len();
    verify(path, size, None, progress).map_err(|err| with_path(err, path))
}

/// Relit `size` octets sans cache, calcule BLAKE3 et xxHash64 et, si
/// `blocks` est fourni, compare chaque bloc de `BLOCK_SIZE` à son hash.
fn verify(
    path: &Path,
    size: u64,
    blocks: Option<&[Hash]>,
    progress: Option<&AtomicU64>,
) -> io::Result<Verified> {
    let (mut reader, uncached) = rawio::open_uncached(path)?;
    let mut buffer = AlignedBuf::new(BLOCK_SIZE as usize);
    let mut full = blake3::Hasher::new();
    let mut xxh = Xxh64::new(0);
    let mut block = blake3::Hasher::new();
    let mut filled = 0u64;
    let mut index = 0usize;

    let outcome = rawio::read_sized(&mut reader, size, &mut buffer, |chunk| {
        full.update(chunk);
        xxh.update(chunk);
        if let Some(progress) = progress {
            progress.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        let Some(expected) = blocks else {
            return Ok(());
        };
        // Les lectures sans cache peuvent rendre moins qu'un bloc : on découpe
        // le flux aux frontières de blocs.
        let mut rest = chunk;
        while !rest.is_empty() {
            let block_len = BLOCK_SIZE.min(size - index as u64 * BLOCK_SIZE);
            let take = ((block_len - filled) as usize).min(rest.len());
            block.update(&rest[..take]);
            filled += take as u64;
            rest = &rest[take..];
            if filled == block_len {
                let Some(hash) = expected.get(index) else {
                    return Err(format!("bloc {index} absent du journal"));
                };
                if block.finalize() != blake3::Hash::from_bytes(*hash) {
                    return Err(format!("le bloc {index} relu ne correspond pas"));
                }
                block = blake3::Hasher::new();
                filled = 0;
                index += 1;
            }
        }
        Ok(())
    });

    match outcome {
        Ok(()) => Ok(Verified {
            blake3: *full.finalize().as_bytes(),
            xxh64: xxh.digest(),
            uncached,
        }),
        Err(ReadError::Io(err)) => Err(err),
        Err(ReadError::Short(read)) => Err(invalid(format!(
            "fichier plus court que prévu ({read} octets sur {size})"
        ))),
        Err(ReadError::Long) => Err(invalid("fichier plus long que prévu".to_string())),
        Err(ReadError::Abort(message)) => Err(invalid(message)),
    }
}

fn with_path(err: io::Error, path: &Path) -> io::Error {
    io::Error::new(err.kind(), format!("{} : {err}", path.display()))
}

/// Octets d'un fichier déjà présents à destination, en lecture seule : la taille
/// entière si le fichier final est là, sinon le point de reprise que donnerait
/// `Incoming::open` (blocs journalisés dont les données sont sur le disque).
/// Sert à afficher dès la reconnexion le même avancement des deux côtés.
pub fn resume_point(base: &Path, entry: &FileEntry) -> u64 {
    if !is_safe_relative_path(&entry.path) {
        return 0;
    }
    let mut final_path = base.to_path_buf();
    for part in entry.path.split('/') {
        final_path.push(part);
    }
    if let Ok(metadata) = fs::metadata(&final_path) {
        return if metadata.is_file() && metadata.len() == entry.size {
            entry.size
        } else {
            0
        };
    }
    let header = journal_header(entry.size, entry.modified_unix);
    let Some(hashes) = read_journal(&with_suffix(&final_path, JOURNAL_SUFFIX), &header) else {
        return 0;
    };
    if hashes.len() as u64 > entry.size.div_ceil(BLOCK_SIZE) {
        return 0;
    }
    let part_len = fs::metadata(with_suffix(&final_path, PART_SUFFIX))
        .map(|m| m.len())
        .unwrap_or(0);
    let mut blocks = hashes.len();
    while blocks > 0 && covered(blocks, entry.size) > part_len {
        blocks -= 1;
    }
    covered(blocks, entry.size)
}

/// Octets couverts par `blocks` blocs complets, sans dépasser la taille du fichier.
fn covered(blocks: usize, size: u64) -> u64 {
    (blocks as u64).saturating_mul(BLOCK_SIZE).min(size)
}

fn journal_header(size: u64, modified_unix: Option<i64>) -> [u8; JOURNAL_HEADER_LEN as usize] {
    let mut header = [0u8; JOURNAL_HEADER_LEN as usize];
    header[..4].copy_from_slice(JOURNAL_MAGIC);
    header[4] = JOURNAL_VERSION;
    header[8..16].copy_from_slice(&size.to_le_bytes());
    header[16..24].copy_from_slice(&modified_unix.unwrap_or(NO_MTIME).to_le_bytes());
    header[24..32].copy_from_slice(&BLOCK_SIZE.to_le_bytes());
    header
}

/// Lit les hash d'un journal existant, si son en-tête correspond au fichier attendu.
fn read_journal(path: &Path, expected_header: &[u8]) -> Option<Vec<Hash>> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() < JOURNAL_HEADER_LEN as usize
        || &bytes[..JOURNAL_HEADER_LEN as usize] != expected_header
    {
        return None;
    }
    let hashes = bytes[JOURNAL_HEADER_LEN as usize..]
        .chunks_exact(HASH_LEN as usize)
        .map(|chunk| {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(chunk);
            hash
        })
        .collect();
    Some(hashes)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut text = path.as_os_str().to_owned();
    text.push(suffix);
    PathBuf::from(text)
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lugus_core::protocol::FileKind;

    /// Dossier de test sur le disque du projet (pas /tmp, souvent en mémoire).
    fn temp_dir(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-tmp")
            .join(format!("incoming-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn entry(len: usize) -> FileEntry {
        FileEntry {
            path: "A001/clip.mov".to_string(),
            size: len as u64,
            modified_unix: Some(1_700_000_000),
            kind: FileKind::Original,
            root_hash: None,
        }
    }

    fn partial(base: &Path, entry: &FileEntry) -> Incoming {
        match Incoming::open(base, entry).unwrap() {
            Opened::Partial(incoming) => incoming,
            Opened::Present(_) => panic!("fichier inattendu"),
        }
    }

    fn send_blocks(incoming: &mut Incoming, data: &[u8], from_block: usize, to_block: usize) {
        for chunk in data
            .chunks(BLOCK_SIZE as usize)
            .enumerate()
            .skip(from_block)
            .take(to_block - from_block)
        {
            let (index, bytes) = chunk;
            let hash = *blake3::hash(bytes).as_bytes();
            incoming
                .write_block(index as u64 * BLOCK_SIZE, bytes, &hash)
                .unwrap();
        }
    }

    #[test]
    fn full_reception() {
        let base = temp_dir("full");
        let data = sample(BLOCK_SIZE as usize * 2 + 1000);
        let entry = entry(data.len());
        let mut incoming = partial(&base, &entry);
        send_blocks(&mut incoming, &data, 0, 3);
        let reread = AtomicU64::new(0);
        let verified = incoming
            .finish(Some(*blake3::hash(&data).as_bytes()), Some(&reread))
            .unwrap();
        // Le compteur de relecture couvre tout le fichier.
        assert_eq!(reread.load(Ordering::Relaxed), data.len() as u64);
        let full = verified.blake3;
        let mut xxh = Xxh64::new(0);
        xxh.update(&data);
        assert_eq!(verified.xxh64, xxh.digest());

        let final_path = base.join("A001/clip.mov");
        assert_eq!(fs::read(&final_path).unwrap(), data);
        assert_eq!(full, *blake3::hash(&data).as_bytes());
        assert!(!with_suffix(&final_path, JOURNAL_SUFFIX).exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn resume_after_interruption() {
        let base = temp_dir("resume");
        let data = sample(BLOCK_SIZE as usize * 2 + 10);
        let entry = entry(data.len());

        let mut first = partial(&base, &entry);
        send_blocks(&mut first, &data, 0, 1);
        drop(first);

        // Le point de reprise lu sans rien modifier est celui de la réouverture.
        assert_eq!(resume_point(&base, &entry), BLOCK_SIZE);
        let mut second = partial(&base, &entry);
        assert_eq!(second.offset(), BLOCK_SIZE);
        send_blocks(&mut second, &data, 1, 3);
        second.finish(None, None).unwrap();
        assert_eq!(fs::read(base.join("A001/clip.mov")).unwrap(), data);
        assert_eq!(resume_point(&base, &entry), data.len() as u64);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn changed_source_restarts_from_zero() {
        let base = temp_dir("changed");
        let data = sample(BLOCK_SIZE as usize + 10);
        let mut first_entry = entry(data.len());
        let mut first = partial(&base, &first_entry);
        send_blocks(&mut first, &data, 0, 1);
        drop(first);

        first_entry.modified_unix = Some(1_800_000_000);
        let second = partial(&base, &first_entry);
        assert_eq!(second.offset(), 0);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn altered_block_is_refused() {
        let base = temp_dir("altered");
        let data = sample(1000);
        let mut incoming = partial(&base, &entry(data.len()));
        let err = incoming.write_block(0, &data, &[0u8; 32]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn disk_corruption_is_detected_on_readback() {
        let base = temp_dir("corrupt");
        let data = sample(5000);
        let mut incoming = partial(&base, &entry(data.len()));
        send_blocks(&mut incoming, &data, 0, 1);

        // Simule une corruption sur le disque après l'écriture.
        let part_path = with_suffix(&base.join("A001/clip.mov"), PART_SUFFIX);
        let mut file = OpenOptions::new().write(true).open(&part_path).unwrap();
        file.write_all(b"X").unwrap();
        drop(file);

        assert!(incoming.finish(None, None).is_err());
        // Après un échec, la nouvelle tentative repart de zéro.
        let retry = partial(&base, &entry(data.len()));
        assert_eq!(retry.offset(), 0);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn empty_file() {
        let base = temp_dir("empty");
        let incoming = partial(&base, &entry(0));
        assert!(incoming.is_complete());
        let verified = incoming
            .finish(Some(*blake3::hash(b"").as_bytes()), None)
            .unwrap();
        assert_eq!(verified.blake3, *blake3::hash(b"").as_bytes());
        // Valeur de référence de xxHash64 pour un fichier vide.
        assert_eq!(verified.xxh64, 0xef46db3751d8e999);
        assert_eq!(fs::read(base.join("A001/clip.mov")).unwrap().len(), 0);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn existing_file_is_reported() {
        let base = temp_dir("present");
        fs::create_dir_all(base.join("A001")).unwrap();
        fs::write(base.join("A001/clip.mov"), b"abc").unwrap();
        assert!(matches!(
            Incoming::open(&base, &entry(3)).unwrap(),
            Opened::Present(_)
        ));
        assert!(Incoming::open(&base, &entry(4)).is_err());
        fs::remove_dir_all(&base).unwrap();
    }
}
