// Masque la console sous Windows en version release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Lugus Transfer by Domingo Olmo Martin : application de bureau.
//!
//! L'interface (`ui/`) parle au moteur par `invoke` et reçoit le déroulement des
//! sessions par l'événement `lugus` (voir `lugus_transfer::session::Event`).
//! Une seule session à la fois, envoi ou réception, annulable.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lugus_core::code::AccessCode;
use lugus_core::link::LugusLink;
use lugus_transfer::session::{
    run_receiver, run_sender, Emit, Event, ReceiveOptions, SendOptions, SenderCommand,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::mpsc;

/// Résultat d'une session, avec l'erreur déjà mise en texte.
type SessionResult = Result<(), String>;

/// Session en cours.
struct Running {
    task: tauri::async_runtime::JoinHandle<()>,
    commands: Option<mpsc::UnboundedSender<SenderCommand>>,
    /// Réception : fichiers choisis par l'utilisateur.
    choices: Option<mpsc::UnboundedSender<Vec<u32>>>,
}

#[derive(Default)]
struct AppState {
    running: Mutex<Option<Running>>,
}

#[derive(serde::Serialize)]
struct PathSummary {
    /// Ce qui est envoyé : le dossier, ou le fichier seul.
    path: String,
    /// Dossier où se lisent les chemins de `entries` (le dossier lui-même, ou le
    /// dossier du fichier). Deux ajouts de même `root` sont fusionnés.
    root: String,
    name: String,
    files: usize,
    bytes: u64,
    /// `true` pour un dossier ou plusieurs éléments : l'interface propose alors
    /// de choisir les fichiers.
    folder: bool,
    /// `true` quand tout un dossier est envoyé (ni fichier seul, ni sélection).
    whole: bool,
    entries: Vec<EntryInfo>,
    /// Plusieurs éléments d'un même dossier : les fichiers concernés, à passer
    /// tels quels à l'envoi (`SourceSpec::only`). Absent : tout `path`.
    only: Option<Vec<String>>,
}

/// Un fichier du dossier, pour l'écran de choix.
#[derive(serde::Serialize)]
struct EntryInfo {
    path: String,
    size: u64,
    kind: lugus_core::protocol::FileKind,
}

/// Choix d'un dossier, en partant du dernier dossier choisi.
#[tauri::command]
async fn pick_folder(app: AppHandle, start: Option<String>) -> Option<String> {
    let mut dialog = app.dialog().file();
    if let Some(start) = start.filter(|s| !s.is_empty()) {
        let path = PathBuf::from(start);
        if path.is_dir() {
            dialog = dialog.set_directory(path);
        }
    }
    dialog.blocking_pick_folder().map(|p| p.to_string())
}

/// Choix d'un ou plusieurs fichiers à envoyer.
#[tauri::command]
async fn pick_files(app: AppHandle) -> Option<Vec<String>> {
    app.dialog()
        .file()
        .blocking_pick_files()
        .map(|files| files.into_iter().map(|p| p.to_string()).collect())
}

/// Résumé de ce qui a été choisi ou glissé avant l'envoi. Chaque dossier, fichier
/// seul ou groupe d'éléments d'un même dossier devient une source ; des éléments
/// de dossiers différents donnent plusieurs sources. Erreur `lugus:` + clé de
/// traduction pour les cas expliqués par l'interface.
#[tauri::command]
async fn scan_paths(paths: Vec<String>) -> Result<Vec<PathSummary>, String> {
    tauri::async_runtime::spawn_blocking(move || summarize(paths))
        .await
        .map_err(|e| e.to_string())?
}

fn summarize(paths: Vec<String>) -> Result<Vec<PathSummary>, String> {
    let items: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    if items.is_empty() {
        return Err("lugus:err_nothing".into());
    }

    // Les éléments d'un même dossier forment une source ; une racine de disque,
    // qui n'a pas de dossier parent, est une source à elle seule.
    let mut groups: Vec<(Option<PathBuf>, Vec<PathBuf>)> = Vec::new();
    for item in items {
        let parent = item
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf);
        match groups
            .iter_mut()
            .find(|(known, _)| parent.is_some() && *known == parent)
        {
            Some((_, members)) => members.push(item),
            None => groups.push((parent, vec![item])),
        }
    }

    groups
        .into_iter()
        .map(|(parent, members)| match (parent, members.as_slice()) {
            (Some(parent), [_, _, ..]) => summarize_parts(&parent, &members),
            (_, [one]) => summarize_one(one),
            _ => Err("lugus:err_nothing".to_string()),
        })
        .collect()
}

/// Un dossier ou un fichier seul.
fn summarize_one(path: &Path) -> Result<PathSummary, String> {
    let manifest = lugus_store::scan(path).map_err(|e| e.to_string())?;
    let root = lugus_store::source_root(path).map_err(|e| e.to_string())?;
    Ok(build_summary(path, &root, manifest, None))
}

/// Plusieurs éléments d'un même dossier.
fn summarize_parts(parent: &Path, items: &[PathBuf]) -> Result<PathSummary, String> {
    let tops: Vec<String> = items
        .iter()
        .filter_map(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    let manifest = lugus_store::scan_parts(parent, &tops).map_err(|e| e.to_string())?;
    let only = manifest.files.iter().map(|f| f.path.clone()).collect();
    Ok(build_summary(parent, parent, manifest, Some(only)))
}

fn build_summary(
    path: &Path,
    root: &Path,
    manifest: lugus_core::protocol::Manifest,
    only: Option<Vec<String>>,
) -> PathSummary {
    PathSummary {
        path: path.display().to_string(),
        root: root.display().to_string(),
        name: manifest.name.clone(),
        files: manifest.files.len(),
        bytes: manifest.total_bytes(),
        folder: path.is_dir(),
        whole: path.is_dir() && only.is_none(),
        entries: manifest
            .files
            .iter()
            .map(|f| EntryInfo {
                path: f.path.clone(),
                size: f.size,
                kind: f.kind,
            })
            .collect(),
        only,
    }
}

/// Vérifie un code en local (forme, mot, chiffre de contrôle). Aucun réseau.
#[tauri::command]
fn check_code(code: String) -> Result<String, String> {
    code.parse::<AccessCode>()
        .map(|c| c.to_string())
        .map_err(|e| e.to_string())
}

/// Vérifie la forme d'un lien.
#[tauri::command]
fn check_link(link: String) -> Result<(), String> {
    link.trim()
        .parse::<LugusLink>()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn emitter(app: &AppHandle) -> Emit {
    let app = app.clone();
    Arc::new(move |event: Event| {
        let _ = app.emit("lugus", &event);
    })
}

/// Lance une session, une seule à la fois. À la fin, l'erreur éventuelle est
/// envoyée à l'interface et la place est libérée.
fn launch<F>(
    app: AppHandle,
    state: State<AppState>,
    commands: Option<mpsc::UnboundedSender<SenderCommand>>,
    choices: Option<mpsc::UnboundedSender<Vec<u32>>>,
    session: F,
) -> Result<(), String>
where
    F: std::future::Future<Output = SessionResult> + Send + 'static,
{
    // La tâche libère la place en fin de session ; elle doit prendre ce même
    // verrou, donc elle ne peut pas le faire avant que `running` soit rempli.
    let mut running = state.running.lock().map_err(|e| e.to_string())?;
    if running.is_some() {
        return Err("Une session est déjà en cours".into());
    }
    let done_app = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        if let Err(message) = session.await {
            let _ = done_app.emit("lugus", &Event::Error { message });
        }
        if let Some(state) = done_app.try_state::<AppState>() {
            if let Ok(mut running) = state.running.lock() {
                *running = None;
            }
        }
    });
    *running = Some(Running {
        task,
        commands,
        choices,
    });
    Ok(())
}

#[tauri::command]
fn start_send(app: AppHandle, state: State<AppState>, options: SendOptions) -> Result<(), String> {
    let (commands, receiver) = mpsc::unbounded_channel();
    let emit = emitter(&app);
    launch(app, state, Some(commands), None, async move {
        run_sender(options, emit, receiver)
            .await
            .map_err(|e| format!("{e:#}"))
    })
}

#[tauri::command]
fn start_receive(
    app: AppHandle,
    state: State<AppState>,
    options: ReceiveOptions,
) -> Result<(), String> {
    let emit = emitter(&app);
    let (choices, picked) = mpsc::unbounded_channel();
    launch(app, state, None, Some(choices), async move {
        run_receiver(options, emit, Some(picked))
            .await
            .map_err(|e| format!("{e:#}"))
    })
}

/// Fichiers choisis pour la réception en cours (indices dans le contenu proposé).
#[tauri::command]
fn choose_files(state: State<AppState>, files: Vec<u32>) -> Result<(), String> {
    let running = state.running.lock().map_err(|e| e.to_string())?;
    match running.as_ref().and_then(|r| r.choices.as_ref()) {
        Some(choices) => choices
            .send(files)
            .map_err(|_| "aucune réception en attente de choix".to_string()),
        None => Err("aucune réception en cours".into()),
    }
}

/// Nouveau code pour l'envoi en cours (le lien ne change pas).
#[tauri::command]
fn new_code(state: State<AppState>) -> Result<(), String> {
    let running = state.running.lock().map_err(|e| e.to_string())?;
    match running.as_ref().and_then(|r| r.commands.as_ref()) {
        Some(commands) => commands
            .send(SenderCommand::NewCode)
            .map_err(|_| "aucun envoi en cours".to_string()),
        None => Err("aucun envoi en cours".into()),
    }
}

/// Arrête la session en cours. Les fichiers partiels et leur journal restent :
/// une nouvelle réception avec le même lien reprendra là où elle s'est arrêtée.
#[tauri::command]
fn cancel(state: State<AppState>) {
    if let Ok(mut running) = state.running.lock() {
        if let Some(running) = running.take() {
            running.task.abort();
        }
    }
}

#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            pick_folder,
            pick_files,
            scan_paths,
            check_code,
            check_link,
            start_send,
            start_receive,
            new_code,
            choose_files,
            cancel,
            open_path,
            reveal_path
        ])
        .run(tauri::generate_context!())
        .expect("Lugus Transfer n'a pas pu démarrer");
}
