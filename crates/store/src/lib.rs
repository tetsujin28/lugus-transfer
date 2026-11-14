//! Fichiers et intégrité.
//!
//! - construction du manifeste à partir d'un fichier ou d'un dossier ;
//! - réception par blocs avec reprise et vérification par relecture sans cache
//!   (modules `incoming` et `rawio`) ;
//! - MHL 1.1 en xxHash64 (module `mhl`), comme Bandua.

pub mod incoming;
pub mod mhl;
pub mod rawio;

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use lugus_core::protocol::{FileEntry, FileKind, Hash, Manifest};

/// Où écrire les fichiers reçus.
pub struct Layout {
    /// Dossier racine des chemins du manifeste (et dossier du MHL).
    pub base: PathBuf,
    /// Préfixe à ajouter aux chemins pour les exprimer depuis la destination.
    pub prefix: String,
}

/// Disposition à destination : un dossier envoyé devient `dest/<nom>/…`, un
/// fichier seul arrive directement dans `dest`.
pub fn layout(dest: &Path, manifest: &Manifest) -> io::Result<Layout> {
    if !is_safe_relative_path(&manifest.name) || manifest.name.contains('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("nom de contenu refusé : {}", manifest.name),
        ));
    }
    let single_file = manifest.files.len() == 1 && manifest.files[0].path == manifest.name;
    if single_file {
        Ok(Layout {
            base: dest.to_path_buf(),
            prefix: String::new(),
        })
    } else {
        Ok(Layout {
            base: dest.join(&manifest.name),
            prefix: format!("{}/", manifest.name),
        })
    }
}

/// Dossier à partir duquel l'émetteur résout les chemins de son manifeste.
pub fn source_root(path: &Path) -> io::Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        Ok(path.to_path_buf())
    } else {
        Ok(path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")))
    }
}

/// Chemin source d'une entrée du manifeste.
pub fn source_path(root: &Path, entry: &FileEntry) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in entry.path.split('/') {
        path.push(part);
    }
    path
}

/// Liste canonique des hash BLAKE3 (`hash  chemin`, triée par chemin) et son
/// propre BLAKE3, qui sert d'empreinte à l'accusé de réception.
pub fn hash_list(hashes: &[(String, Hash)]) -> (String, Hash) {
    let mut sorted: Vec<&(String, Hash)> = hashes.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut content = String::new();
    for (path, hash) in sorted {
        let _ = writeln!(content, "{}  {path}", hex(hash));
    }
    let digest = *blake3::hash(content.as_bytes()).as_bytes();
    (content, digest)
}

/// Nom du MHL écrit par Lugus, distinct de celui de Bandua pour ne jamais
/// écraser un MHL déjà présent dans le dossier reçu.
pub fn mhl_file_name(manifest_name: &str, when: &chrono::DateTime<chrono::Local>) -> String {
    format!(
        "{manifest_name}_lugus_{}.mhl",
        when.format("%Y-%m-%d_%H%M%S")
    )
}

/// Hash en hexadécimal minuscule.
pub fn hex(hash: &Hash) -> String {
    hash.iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Extensions considérées comme des fichiers vidéo ou des images de caméra.
const MEDIA_EXTENSIONS: &[&str] = &[
    "mov", "mp4", "mxf", "braw", "r3d", "ari", "arx", "crm", "avi", "mkv", "dng", "mts", "m2ts",
];

/// Noms de fichiers système à ignorer.
/// Fichiers et dossiers système à ignorer (y compris à la racine d'un disque Windows).
const IGNORED_NAMES: &[&str] = &[
    "Thumbs.db",
    "desktop.ini",
    "System Volume Information",
    "$RECYCLE.BIN",
    "$Recycle.Bin",
];

/// Décrit un fichier ou un dossier (récursivement) sous forme de manifeste.
///
/// - les chemins sont relatifs, avec `/` comme séparateur ;
/// - les fichiers cachés (commençant par `.`), les fichiers système et les liens
///   symboliques sont ignorés ;
/// - les fichiers sont triés par chemin ;
/// - les hash ne sont pas calculés ici (`root_hash` reste vide).
pub fn scan(path: &Path) -> io::Result<Manifest> {
    let metadata = fs::symlink_metadata(path)?;
    let name = content_name(path);

    let mut files = Vec::new();
    if metadata.is_file() {
        // Le chemin relatif d'un fichier seul est son vrai nom, pour le relire.
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());
        files.push(entry(file_name, &metadata));
    } else if metadata.is_dir() {
        walk(path, path, &mut files)?;
        files.sort_by(|a, b| a.path.cmp(&b.path));
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ni un fichier ni un dossier",
        ));
    }

    Ok(Manifest { name, files })
}

/// Nom du contenu tel que le verra le destinataire : le nom du dossier ou du
/// fichier, sans caractère interdit. Une racine de disque (`G:\` sous Windows,
/// `/` ailleurs) n'a pas de nom : elle devient `Disque_G`.
pub fn content_name(path: &Path) -> String {
    if let Some(name) = path.file_name() {
        let clean = sanitize_name(&name.to_string_lossy());
        if !clean.is_empty() {
            return clean;
        }
    }
    let letters: String = path
        .to_string_lossy()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if letters.is_empty() {
        "Disque".to_string()
    } else {
        format!("Disque_{letters}")
    }
}

/// Remplace les caractères interdits dans un nom de fichier (Windows compris)
/// et retire les espaces et points en début ou fin de nom.
fn sanitize_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced.trim_matches(|c| c == ' ' || c == '.');
    if trimmed.chars().all(|c| c == '_') {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn walk(root: &Path, dir: &Path, files: &mut Vec<FileEntry>) -> io::Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let file_name = item.file_name().to_string_lossy().into_owned();
        if is_ignored(&file_name) {
            continue;
        }
        let path = item.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            walk(root, &path, files)?;
        } else if metadata.is_file() {
            if let Some(relative) = relative_path(root, &path) {
                files.push(entry(relative, &metadata));
            }
        }
        // Les liens symboliques et fichiers spéciaux sont ignorés.
    }
    Ok(())
}

fn entry(relative: String, metadata: &fs::Metadata) -> FileEntry {
    let modified_unix = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_secs()).ok());
    FileEntry {
        kind: classify(&relative),
        path: relative,
        size: metadata.len(),
        modified_unix,
        root_hash: None,
    }
}

fn is_ignored(file_name: &str) -> bool {
    file_name.starts_with('.')
        || IGNORED_NAMES.contains(&file_name)
        || file_name.ends_with(".lugus-part")
        || file_name.ends_with(".lugus-journal")
}

/// Chemin relatif avec `/`, uniquement fait de composants normaux.
fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Classement simple : proxy si le chemin le dit, original si c'est un média,
/// fichier annexe sinon (XML, LUT, audio, rapports…).
fn classify(relative: &str) -> FileKind {
    let lower = relative.to_ascii_lowercase();
    let extension = lower.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    if !MEDIA_EXTENSIONS.contains(&extension) {
        return FileKind::Sidecar;
    }
    if lower.contains("proxy") || lower.contains("proxies") {
        FileKind::Proxy
    } else {
        FileKind::Original
    }
}

/// Vérifie qu'un chemin reçu dans un manifeste est sûr à écrire sous le dossier
/// de destination : relatif, sans `..`, sans racine ni lecteur.
pub fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_files() {
        assert_eq!(classify("A001/A001C003.MOV"), FileKind::Original);
        assert_eq!(classify("Proxy/A001C003.mov"), FileKind::Proxy);
        assert_eq!(classify("A001/A001C003_proxy.mp4"), FileKind::Proxy);
        assert_eq!(classify("A001/A001C003.xml"), FileKind::Sidecar);
        assert_eq!(classify("LUT/show.cube"), FileKind::Sidecar);
        assert_eq!(classify("README"), FileKind::Sidecar);
    }

    #[test]
    fn layout_for_folder_and_single_file() {
        let folder = Manifest {
            name: "J04".to_string(),
            files: vec![],
        };
        let layout_folder = layout(Path::new("/dest"), &folder).unwrap();
        assert_eq!(layout_folder.base, Path::new("/dest").join("J04"));
        assert_eq!(layout_folder.prefix, "J04/");

        let single = Manifest {
            name: "clip.mov".to_string(),
            files: vec![FileEntry {
                path: "clip.mov".to_string(),
                size: 1,
                modified_unix: None,
                kind: FileKind::Original,
                root_hash: None,
            }],
        };
        let layout_single = layout(Path::new("/dest"), &single).unwrap();
        assert_eq!(layout_single.base, Path::new("/dest"));
        assert_eq!(layout_single.prefix, "");

        let evil = Manifest {
            name: "..".to_string(),
            files: vec![],
        };
        assert!(layout(Path::new("/dest"), &evil).is_err());
    }

    #[test]
    fn content_names() {
        assert_eq!(content_name(Path::new("/rushes/J04")), "J04");
        assert_eq!(content_name(Path::new("/rushes/clip.braw")), "clip.braw");
        assert_eq!(sanitize_name("J04: jour 4?"), "J04_ jour 4_");
        assert_eq!(sanitize_name(" ..carte. "), "carte");
        assert_eq!(sanitize_name(":"), "");
        // Racine : jamais de nom vide ni de caractère interdit.
        let root = content_name(Path::new("/"));
        assert!(is_safe_relative_path(&root) && !root.contains('/'));
        assert!(root.starts_with("Disque"));
    }

    #[test]
    fn system_folders_are_ignored() {
        assert!(is_ignored("System Volume Information"));
        assert!(is_ignored("$RECYCLE.BIN"));
        assert!(is_ignored(".Spotlight-V100"));
        assert!(!is_ignored("A001"));
    }

    #[test]
    fn hex_format() {
        let mut hash = [0u8; 32];
        hash[0] = 0xab;
        hash[31] = 0x01;
        let text = hex(&hash);
        assert_eq!(text.len(), 64);
        assert!(text.starts_with("ab00"));
        assert!(text.ends_with("01"));
    }

    #[test]
    fn safe_paths() {
        assert!(is_safe_relative_path("A001/clip.mov"));
        assert!(!is_safe_relative_path("../etc/passwd"));
        assert!(!is_safe_relative_path("A001/../../x"));
        assert!(!is_safe_relative_path("/etc/passwd"));
        assert!(!is_safe_relative_path("C:/Windows"));
        assert!(!is_safe_relative_path("A001\\clip.mov"));
        assert!(!is_safe_relative_path("A001//clip.mov"));
        assert!(!is_safe_relative_path(""));
    }

    #[test]
    fn scan_directory() {
        let root = std::env::temp_dir().join(format!("lugus-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("A001")).unwrap();
        fs::write(root.join("A001/clip.mov"), b"0123456789").unwrap();
        fs::write(root.join("A001/clip.xml"), b"<x/>").unwrap();
        fs::write(root.join(".DS_Store"), b"ignore").unwrap();

        let manifest = scan(&root).unwrap();
        let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["A001/clip.mov", "A001/clip.xml"]);
        assert_eq!(manifest.total_bytes(), 14);
        assert_eq!(manifest.files[0].kind, FileKind::Original);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scan_single_file() {
        let path = std::env::temp_dir().join(format!("lugus-single-{}.mov", std::process::id()));
        fs::write(&path, b"abc").unwrap();
        let manifest = scan(&path).unwrap();
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].size, 3);
        fs::remove_file(&path).unwrap();
    }
}
