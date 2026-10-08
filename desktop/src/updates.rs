use std::{sync::atomic::Ordering, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use twitch_drops_miner_core::app::AppError;

use crate::{Desktop, ipc};

const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Installing,
    Failed,
}

#[derive(Clone, Serialize)]
pub struct Status {
    revision: u64,
    phase: Phase,
    current_version: &'static str,
    version: Option<String>,
    downloaded: u64,
    total: Option<u64>,
    error: Option<&'static str>,
    restart_required: bool,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: Phase::Idle,
            current_version: env!("CARGO_PKG_VERSION"),
            version: None,
            downloaded: 0,
            total: None,
            error: None,
            restart_required: false,
        }
    }
}

#[derive(Default)]
struct Pending {
    status: Status,
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
    cancel: Option<CancellationToken>,
}

#[derive(Default)]
pub struct Updates {
    pending: Mutex<Pending>,
    pub tasks: TaskTracker,
    pub cancel: CancellationToken,
}

impl Pending {
    fn publish(&mut self, app: &tauri::AppHandle) -> Status {
        self.status.revision += 1;
        let _ = app.emit_to("main", "desktop-update", &self.status);
        self.status.clone()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Check,
    Download,
    Cancel,
    Install,
}

#[tauri::command]
pub async fn desktop_update(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, Desktop>,
    action: Option<Action>,
) -> Result<Status, ipc::Error> {
    ipc::local(&window)?;
    let Some(action) = action else {
        return Ok(state.updates.pending.lock().await.status.clone());
    };
    start(&app, action).await
}

pub async fn start(app: &tauri::AppHandle, action: Action) -> Result<Status, ipc::Error> {
    let state = app.state::<Desktop>();
    let mut pending = state.updates.pending.lock().await;
    if matches!(action, Action::Cancel) {
        if let Some(cancel) = pending.cancel.as_ref() {
            cancel.cancel();
        }
        return Ok(pending.status.clone());
    }
    if matches!(
        pending.status.phase,
        Phase::Checking | Phase::Downloading | Phase::Installing
    ) || state.quitting.load(Ordering::SeqCst)
        || pending.status.restart_required
    {
        return Err(AppError::Unavailable.into());
    }
    pending.status.error = None;
    let cancel = state.updates.cancel.child_token();
    pending.cancel = Some(cancel.clone());
    let handle = app.clone();
    match action {
        Action::Check => {
            pending.status.phase = Phase::Checking;
            pending.status.version = None;
            pending.update = None;
            pending.bytes = None;
            state.updates.tasks.spawn(async move {
                check(handle, cancel).await;
            });
        }
        Action::Download => {
            let update = pending.update.clone().ok_or(AppError::InvalidRequest)?;
            pending.status.phase = Phase::Downloading;
            pending.status.downloaded = 0;
            pending.status.total = None;
            state.updates.tasks.spawn(async move {
                download(handle, update, cancel).await;
            });
        }
        Action::Install => {
            if pending.status.phase != Phase::Ready
                || pending.bytes.is_none()
                || pending.update.is_none()
            {
                return Err(AppError::InvalidRequest.into());
            }
            if state.quitting.swap(true, Ordering::SeqCst) {
                return Err(AppError::ShuttingDown.into());
            }
            let update = pending.update.take().ok_or(AppError::InvalidRequest)?;
            let bytes = pending.bytes.take().ok_or(AppError::InvalidRequest)?;
            state.installing.store(true, Ordering::SeqCst);
            pending.status.phase = Phase::Installing;
            state.updates.tasks.spawn(async move {
                let state = handle.state::<Desktop>();
                state.drain().await;
                // The Windows updater exits the process. All mining and disk work is already drained.
                let result =
                    tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await;
                state.installing.store(false, Ordering::SeqCst);
                if matches!(result, Ok(Ok(()))) {
                    handle.restart();
                }
                let mut pending = state.updates.pending.lock().await;
                pending.status.phase = Phase::Failed;
                pending.status.error = Some("install_failed");
                pending.status.restart_required = true;
                pending.cancel = None;
                pending.publish(&handle);
                state.quitting.store(false, Ordering::SeqCst);
                crate::show(&handle);
            });
        }
        Action::Cancel => unreachable!(),
    }
    Ok(pending.publish(app))
}

async fn check(app: tauri::AppHandle, cancel: CancellationToken) {
    let operation = async {
        let updater = app
            .updater_builder()
            .timeout(Duration::from_secs(600))
            .build()?;
        updater.check().await
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        result = tokio::time::timeout(Duration::from_secs(20), operation) => Some(result),
    };
    let state = app.state::<Desktop>();
    let mut pending = state.updates.pending.lock().await;
    match result {
        Some(Ok(Ok(Some(update)))) if valid_download(&update.download_url, &update.version) => {
            pending.status.version = Some(update.version.clone());
            pending.status.phase = Phase::Available;
            pending.update = Some(update);
        }
        Some(Ok(Ok(None))) => pending.status.phase = Phase::Current,
        None => pending.status.phase = Phase::Idle,
        _ => {
            pending.status.phase = Phase::Failed;
            pending.status.error = Some("check_failed");
        }
    }
    pending.cancel = None;
    pending.publish(&app);
}

fn valid_download(url: &url::Url, version: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url
            .path()
            .strip_prefix(&format!(
                "/ohne-b/twitch-drops-miner/releases/download/v{version}/"
            ))
            .is_some_and(|file| !file.is_empty() && !file.contains('/') && !file.contains('%'))
}

async fn download(app: tauri::AppHandle, update: Update, cancel: CancellationToken) {
    let mut downloaded = 0u64;
    let mut progress = None;
    let mut oversized = false;
    let operation = update.download(
        |size, total| {
            downloaded = downloaded.saturating_add(size as u64);
            if downloaded > MAX_DOWNLOAD || total.is_some_and(|size| size > MAX_DOWNLOAD) {
                oversized = true;
                cancel.cancel();
                return;
            }
            let next = total
                .filter(|v| *v > 0)
                .map_or(downloaded / (1024 * 1024), |v| {
                    downloaded.saturating_mul(100) / v
                });
            if progress != Some(next) {
                progress = Some(next);
                // The callback is synchronous; this lock never crosses an await in the downloader.
                if let Ok(mut pending) = app.state::<Desktop>().updates.pending.try_lock() {
                    pending.status.downloaded = downloaded;
                    pending.status.total = total;
                    pending.publish(&app);
                }
            }
        },
        || {},
    );
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        result = operation => Some(result),
    };
    let state = app.state::<Desktop>();
    let mut pending = state.updates.pending.lock().await;
    match result {
        Some(Ok(bytes)) if !oversized && !cancel.is_cancelled() => {
            pending.status.phase = Phase::Ready;
            pending.status.downloaded = bytes.len() as u64;
            pending.bytes = Some(bytes);
        }
        None if !oversized => pending.status.phase = Phase::Available,
        _ => {
            pending.status.phase = Phase::Failed;
            pending.status.error = Some("download_failed");
        }
    }
    pending.cancel = None;
    pending.publish(&app);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installer_links_are_confined_to_the_announced_release() {
        let good = "https://github.com/ohne-b/twitch-drops-miner/releases/download/v1.7.0/Drops-Miner-setup.exe";
        assert!(valid_download(&url::Url::parse(good).unwrap(), "1.7.0"));
        for bad in [
            good.replace("github.com", "example.com"),
            good.replace("https:", "http:"),
            good.replace("v1.7.0", "v1.6.0"),
            format!("{good}?redirect=elsewhere"),
            good.replace("github.com", "user@github.com"),
        ] {
            assert!(!valid_download(&url::Url::parse(&bad).unwrap(), "1.7.0"));
        }
    }
}
