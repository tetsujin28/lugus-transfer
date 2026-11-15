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

/// Manifeste de quelques éléments du dossier `root`, désignés par leur nom :
/// fichiers ou sous-dossiers (parcourus récursivement). Seuls ces éléments sont
/// lus, pas le reste du dossier. Le nom du contenu est celui de `root`.
pub fn scan_parts(root: &Path, tops: &[String]) -> io::Result<Manifest> {
    let mut files = Vec::new();
    for top in tops {
        if !is_safe_relative_path(top) || top.contains('/') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("nom refusé : {top}"),
            ));
        }
        if is_ignored(top) {
            continue;
        }
        let path = root.join(top);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            walk(root, &path, &mut files)?;
        } else if metadata.is_file() {
            files.push(entry(top.clone(), &metadata));
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    Ok(Manifest {
        name: content_name(root),
        files,
    })
}

/// Premier niveau de chaque chemin (`A001/clip.mov` donne `A001`), sans doublon.
pub fn top_levels(paths: &[String]) -> Vec<String> {
    let mut tops: Vec<String> = paths
        .iter()
        .filter_map(|p| p.split('/').next())
        .map(str::to_string)
        .collect();
    tops.sort();
    tops.dedup();
    tops
}

/// Ne garde du manifeste que les chemins choisis par l'expéditeur, dans l'ordre
/// du manifeste. Erreur si la liste est vide ou si un chemin n'existe plus (le
/// dossier a changé entre le choix et l'envoi).
pub fn keep_only(manifest: Manifest, only: &[String]) -> io::Result<Manifest> {
    if only.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "aucun fichier choisi",
        ));
    }
    let wanted: std::collections::HashSet<&str> = only.iter().map(String::as_str).collect();
    let files: Vec<FileEntry> = manifest
        .files
        .into_iter()
        .filter(|f| wanted.contains(f.path.as_str()))
        .collect();
    if files.len() != wanted.len() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "des fichiers choisis ont disparu ou changé de nom : choisis à nouveau le dossier",
        ));
    }
    Ok(Manifest {
        name: manifest.name,
        files,
    })
}

/// Nom du contenu tel que le verra le destinataire : le nom du dossier ou du
/// fichier, sans caractère interdit. Une racine de disque n'a pas de nom : on
/// prend le nom du volume (sous Windows, celui affiché dans l'Explorateur ; sur
/// Mac, une carte est montée sous `/Volumes/<nom>` et a donc déjà un nom). Un
/// disque sans nom devient `Rushes_<date d'envoi>` : la lettre du lecteur ne
/// voudrait rien dire chez le destinataire.
pub fn content_name(path: &Path) -> String {
    if let Some(name) = path.file_name() {
        let clean = sanitize_name(&name.to_string_lossy());
        if !clean.is_empty() {
            return clean;
        }
    }
    if let Some(label) = volume_label(path) {
        let clean = sanitize_name(&label);
        if !clean.is_empty() {
            return clean;
        }
    }
    format!("Rushes_{}", chrono::Local::now().format("%Y-%m-%d"))
}

/// Nom du volume dont `path` est la racine (Windows uniquement).
#[cfg(windows)]
fn volume_label(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;

    // La fonction attend une racine terminée par une barre oblique inverse : « G:\ ».
    let backslash = u16::from(b'\\');
    let mut root: Vec<u16> = path.as_os_str().encode_wide().collect();
    if root.last() != Some(&backslash) {
        root.push(backslash);
    }
    root.push(0);

    let mut name = [0u16; 261];
    // SAFETY : chaînes terminées par zéro, tampon de taille annoncée, pointeurs
    // facultatifs nuls (autorisés par l'API).
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            name.as_mut_ptr(),
            name.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    let label = String::from_utf16_lossy(&name[..len]);
    let label = label.trim();
    (!label.is_empty()).then(|| label.to_string())
}

#[cfg(not(windows))]
fn volume_label(_path: &Path) -> Option<String> {
    None
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
        assert!(root.starts_with("Rushes_20"));
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
    fn keep_only_selected_paths() {
        let file = |path: &str| FileEntry {
            path: path.to_string(),
            size: 1,
            modified_unix: None,
            kind: classify(path),
            root_hash: None,
        };
        let manifest = Manifest {
            name: "J04".to_string(),
            files: vec![
                file("A001/clip.braw"),
                file("A001/clip.xml"),
                file("Proxy/clip.mov"),
            ],
        };
        let only = vec!["Proxy/clip.mov".to_string(), "A001/clip.braw".to_string()];
        let kept = keep_only(manifest.clone(), &only).unwrap();
        let paths: Vec<&str> = kept.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["A001/clip.braw", "Proxy/clip.mov"]);
        assert_eq!(kept.name, "J04");

        assert!(keep_only(manifest.clone(), &[]).is_err());
        let gone = vec!["A001/absent.braw".to_string()];
        assert!(keep_only(manifest, &gone).is_err());
    }

    #[test]
    fn scan_some_parts_only() {
        let root = std::env::temp_dir().join(format!("lugus-parts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Proxy/A001")).unwrap();
        fs::create_dir_all(root.join("Autre")).unwrap();
        fs::write(root.join("A001C001.braw"), b"0123").unwrap();
        fs::write(root.join("A001C002.braw"), b"45").unwrap();
        fs::write(root.join("Proxy/A001/A001C001.mov"), b"6").unwrap();
        fs::write(root.join("Autre/ignore.txt"), b"7").unwrap();

        let tops = vec!["A001C001.braw".to_string(), "Proxy".to_string()];
        let manifest = scan_parts(&root, &tops).unwrap();
        let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["A001C001.braw", "Proxy/A001/A001C001.mov"]);
        assert!(scan_parts(&root, &["../x".to_string()]).is_err());

        let only = vec![
            "Proxy/A001/A001C001.mov".to_string(),
            "A001C001.braw".to_string(),
        ];
        assert_eq!(top_levels(&only), ["A001C001.braw", "Proxy"]);
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
