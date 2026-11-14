// Masque la console sous Windows en version release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Lugus Transfer by Domingo Olmo Martin : application de bureau.
//!
//! L'interface (`ui/`) parle au moteur par `invoke` et reçoit le déroulement des
//! sessions par l'événement `lugus` (voir `lugus_transfer::session::Event`).
//! Une seule session à la fois, envoi ou réception, annulable.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lugus_core::code::AccessCode;
use lugus_core::link::LugusLink;
use lugus_transfer::session::{
    run_receiver, run_sender, Emit, Event, ReceiveOptions, SendOptions, SenderCommand,
};
use qrcode::render::svg;
use qrcode::QrCode;
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
}

#[derive(Default)]
struct AppState {
    running: Mutex<Option<Running>>,
}

#[derive(serde::Serialize)]
struct PathSummary {
    path: String,
    name: String,
    files: usize,
    bytes: u64,
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

/// Choix d'un fichier seul à envoyer.
#[tauri::command]
async fn pick_file(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .blocking_pick_file()
        .map(|p| p.to_string())
}

/// Résumé d'un fichier ou d'un dossier avant l'envoi.
#[tauri::command]
async fn scan_path(path: String) -> Result<PathSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let manifest = lugus_store::scan(&PathBuf::from(&path)).map_err(|e| e.to_string())?;
        Ok(PathSummary {
            name: manifest.name.clone(),
            files: manifest.files.len(),
            bytes: manifest.total_bytes(),
            path,
        })
    })
    .await
    .map_err(|e| e.to_string())?
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

/// QR code du lien, en SVG, aux couleurs de l'interface.
#[tauri::command]
fn qr_svg(text: String) -> Result<String, String> {
    let code = QrCode::new(text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .dark_color(svg::Color("#e7e9ec"))
        .light_color(svg::Color("#141619"))
        .build())
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
    *running = Some(Running { task, commands });
    Ok(())
}

#[tauri::command]
fn start_send(app: AppHandle, state: State<AppState>, options: SendOptions) -> Result<(), String> {
    let (commands, receiver) = mpsc::unbounded_channel();
    let emit = emitter(&app);
    launch(app, state, Some(commands), async move {
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
    launch(app, state, None, async move {
        run_receiver(options, emit)
            .await
            .map_err(|e| format!("{e:#}"))
    })
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
            pick_file,
            scan_path,
            check_code,
            check_link,
            qr_svg,
            start_send,
            start_receive,
            new_code,
            cancel,
            open_path,
            reveal_path
        ])
        .run(tauri::generate_context!())
        .expect("Lugus Transfer n'a pas pu démarrer");
}
