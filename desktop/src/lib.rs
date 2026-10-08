mod ipc;
mod shell;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tauri::Manager;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use twitch_drops_miner_core::{
    app::{AppError, Application},
    logging,
    runtime::Runtime,
};

struct Desktop {
    runtime: Mutex<Option<Runtime>>,
    startup_error: Option<String>,
    requests: Semaphore,
    subscription: Mutex<Option<Arc<ipc::Subscription>>>,
    quitting: AtomicBool,
    drained: AtomicBool,
    cancel: CancellationToken,
    tasks: TaskTracker,
    pending: Mutex<HashMap<String, CancellationToken>>,
    preferences: Mutex<shell::Preferences>,
    preferences_path: PathBuf,
    tray_available: AtomicBool,
    close_to_tray: AtomicBool,
    pause_action: Mutex<()>,
}

fn local_url(url: &url::Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let bundled = matches!(
        (url.scheme(), url.host_str()),
        ("tauri", Some("localhost")) | ("http" | "https", Some("tauri.localhost"))
    ) && url.port().is_none();
    bundled
        || (cfg!(debug_assertions) && url.origin().ascii_serialization() == "http://127.0.0.1:5173")
}

fn open_external(app: &tauri::AppHandle, url: &url::Url) {
    if matches!(url.scheme(), "https" | "http")
        && url.username().is_empty()
        && url.password().is_none()
    {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
}

#[tauri::command]
fn quit_app(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), ipc::Error> {
    ipc::local(&window)?;
    quit(&app);
    Ok(())
}

#[tauri::command]
fn restart_app(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), ipc::Error> {
    ipc::local(&window)?;
    finish(&app, true);
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Folder {
    Data,
    Logs,
}

#[tauri::command]
fn open_app_folder(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    folder: Folder,
) -> Result<(), ipc::Error> {
    ipc::local(&window)?;
    let path = match folder {
        Folder::Data => app.path().app_local_data_dir(),
        Folder::Logs => app.path().app_log_dir(),
    }
    .map_err(|_| AppError::Unavailable)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|_| AppError::Unavailable)?;
    Ok(())
}

impl Desktop {
    async fn application(&self) -> Result<Arc<Application>, ipc::Error> {
        let runtime = self.runtime.lock().await;
        runtime
            .as_ref()
            .map(|value| value.app.clone())
            .ok_or_else(|| ipc::Error {
                status: 503,
                detail: self
                    .startup_error
                    .clone()
                    .unwrap_or_else(|| AppError::Unavailable.to_string()),
            })
    }
}

fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn quit(app: &tauri::AppHandle) {
    finish(app, false);
}

fn finish(app: &tauri::AppHandle, restart: bool) {
    let state = app.state::<Desktop>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = handle.state::<Desktop>();
        state.cancel.cancel();
        state.tasks.close();
        if let Some(mut runtime) = state.runtime.lock().await.take() {
            if runtime.shutdown().await.is_err() {
                tracing::error!("Mining task failed during shutdown");
            }
        }
        state.tasks.wait().await;
        state.drained.store(true, Ordering::SeqCst);
        if restart {
            handle.restart();
        } else {
            handle.exit(0);
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        & !tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg("--autostart")
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let directory = app.path().app_local_data_dir()?;
            let logs = app.path().app_log_dir()?;
            let preferences_path = directory.join("desktop.json");
            let preferences =
                twitch_drops_miner_core::store::read_json::<shell::Preferences>(&preferences_path);
            let guard = logging::initialize(&logs, 0, false);
            let opened = if preferences.is_ok() && guard.is_ok() {
                Application::open(directory.join("data"))
            } else {
                Err(anyhow::anyhow!("desktop storage unavailable"))
            };
            app.manage(Mutex::new(guard.ok()));
            let preferences = preferences.ok().flatten().unwrap_or_default();
            let start_minimized = preferences.start_minimized;
            let close_to_tray = preferences.close_to_tray;
            let (runtime, startup_error) = match opened {
                Ok((application, commands)) => (
                    Some(tauri::async_runtime::block_on(async {
                        Runtime::start(application, commands)
                    })),
                    None,
                ),
                Err(_) => (None, Some("desktop_start_failed".into())),
            };
            app.manage(Desktop {
                runtime: Mutex::new(runtime),
                startup_error,
                requests: Semaphore::new(8),
                subscription: Mutex::new(None),
                quitting: AtomicBool::new(false),
                drained: AtomicBool::new(false),
                cancel: CancellationToken::new(),
                tasks: TaskTracker::new(),
                pending: Mutex::new(HashMap::new()),
                preferences: Mutex::new(preferences),
                preferences_path,
                tray_available: AtomicBool::new(false),
                close_to_tray: AtomicBool::new(close_to_tray),
                pause_action: Mutex::new(()),
            });
            let handle = app.handle().clone();
            let popup_handle = handle.clone();
            let window =
                tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                    .visible(false)
                    .on_navigation(move |url| {
                        if local_url(url) {
                            return true;
                        }
                        open_external(&handle, url);
                        false
                    })
                    .on_new_window(move |url, _| {
                        open_external(&popup_handle, &url);
                        tauri::webview::NewWindowResponse::Deny
                    })
                    .build()?;
            if shell::install(app.handle()).is_err() {
                tracing::warn!("Tray icon unavailable; keeping the window accessible");
            }
            let state = app.state::<Desktop>();
            if !start_minimized
                || !state.tray_available.load(Ordering::SeqCst)
                || state.startup_error.is_some()
            {
                window.show()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::app_request,
            ipc::cancel_request,
            quit_app,
            restart_app,
            open_app_folder,
            shell::desktop_settings,
            ipc::state_open,
            ipc::state_next,
            ipc::state_close
        ])
        .build(tauri::generate_context!())
        .expect("could not initialize Drops Miner")
        .run(|app, event| {
            if let tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } = &event
            {
                if label == "main" {
                    let state = app.state::<Desktop>();
                    if state.tray_available.load(Ordering::SeqCst)
                        && !state.quitting.load(Ordering::SeqCst)
                        && state.close_to_tray.load(Ordering::SeqCst)
                    {
                        api.prevent_close();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                }
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !app.state::<Desktop>().drained.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    quit(app);
                }
            }
        });
}
