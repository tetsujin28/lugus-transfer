//! Fichiers et intégrité.
//!
//! Pour l'instant : la construction du manifeste à partir d'un fichier ou d'un
//! dossier. Viendront ensuite le hashage BLAKE3 en flux, la lecture et l'écriture
//! par blocs vérifiés, le journal de reprise, le MHL et l'accusé de réception.

use std::fs;
use std::io;
use std::path::{Component, Path};
use std::time::UNIX_EPOCH;

use lugus_core::protocol::{FileEntry, FileKind, Manifest};

/// Extensions considérées comme des fichiers vidéo ou des images de caméra.
const MEDIA_EXTENSIONS: &[&str] = &[
    "mov", "mp4", "mxf", "braw", "r3d", "ari", "arx", "crm", "avi", "mkv", "dng", "mts", "m2ts",
];

/// Noms de fichiers système à ignorer.
const IGNORED_NAMES: &[&str] = &["Thumbs.db", "desktop.ini"];

/// Décrit un fichier ou un dossier (récursivement) sous forme de manifeste.
///
/// - les chemins sont relatifs, avec `/` comme séparateur ;
/// - les fichiers cachés (commençant par `.`), les fichiers système et les liens
///   symboliques sont ignorés ;
/// - les fichiers sont triés par chemin ;
/// - les hash ne sont pas calculés ici (`root_hash` reste vide).
pub fn scan(path: &Path) -> io::Result<Manifest> {
    let metadata = fs::symlink_metadata(path)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let mut files = Vec::new();
    if metadata.is_file() {
        files.push(entry(name.clone(), &metadata));
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
    file_name.starts_with('.') || IGNORED_NAMES.contains(&file_name)
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
