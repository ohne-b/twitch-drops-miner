use std::sync::atomic::Ordering;

use tauri::{Emitter, Manager};
use tokio::sync::watch;
use twitch_drops_miner_core::dto::{MiningState, Snapshot};

use crate::Desktop;

pub fn needed(enabled: bool, snapshot: &Snapshot) -> bool {
    enabled
        && snapshot.login.user_id.is_some()
        && !snapshot.settings.values.mining_paused
        && matches!(
            snapshot.mining.state,
            MiningState::Watching | MiningState::AwaitingProgress | MiningState::ManualWatching
        )
}

pub fn start(app: &tauri::AppHandle) -> watch::Sender<bool> {
    let (sender, receiver) = watch::channel(false);
    let handle = app.clone();
    app.state::<Desktop>().tasks.spawn_blocking(move || {
        run(receiver, acquire, |failed| {
            let state = handle.state::<Desktop>();
            if state.keep_awake_failed.swap(failed, Ordering::SeqCst) != failed {
                if failed {
                    tracing::warn!("Could not prevent idle sleep; the system may not support it");
                }
                let _ = handle.emit_to("main", "desktop-keep-awake-failed", failed);
            }
        });
    });
    sender
}

#[cfg(not(any(test, feature = "desktop-fixture")))]
fn acquire() -> Result<keepawake::KeepAwake, keepawake::Error> {
    keepawake::Builder::default()
        .display(false)
        .idle(true)
        .sleep(false)
        .app_name("Drops Miner")
        .app_reverse_domain("dev.ohneb.dropsminer")
        .reason("Mining Twitch drops")
        .create()
}

#[cfg(any(test, feature = "desktop-fixture"))]
fn acquire() -> Result<(), std::convert::Infallible> {
    Ok(())
}

fn run<T, E>(
    mut receiver: watch::Receiver<bool>,
    mut acquire: impl FnMut() -> Result<T, E>,
    mut report: impl FnMut(bool),
) {
    // Windows requires acquisition and release on the same thread, including shutdown.
    let runtime = tokio::runtime::Handle::current();
    let mut guard = None;
    while runtime.block_on(receiver.changed()).is_ok() {
        let wanted = *receiver.borrow_and_update();
        if wanted && guard.is_none() {
            match acquire() {
                Ok(value) => {
                    guard = Some(value);
                    report(false);
                }
                Err(_) => report(true),
            }
        } else if !wanted {
            guard = None;
            report(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, mpsc};

    #[test]
    fn only_active_unpaused_account_mining_prevents_idle_sleep() {
        let mut snapshot: Snapshot =
            serde_json::from_str(include_str!("../../frontend/tests/fixture.json")).unwrap();
        for state in [
            MiningState::Unknown,
            MiningState::AccountRequired,
            MiningState::Paused,
            MiningState::NoSelection,
            MiningState::Discovering,
            MiningState::Watching,
            MiningState::AwaitingClaim,
            MiningState::AwaitingProgress,
            MiningState::WaitingChannel,
            MiningState::NoRewards,
            MiningState::ManualOffline,
            MiningState::ManualWatching,
        ] {
            snapshot.mining.state = state;
            assert_eq!(
                needed(true, &snapshot),
                matches!(
                    state,
                    MiningState::Watching
                        | MiningState::AwaitingProgress
                        | MiningState::ManualWatching
                ),
                "{state:?}"
            );
            assert!(!needed(false, &snapshot));
        }
        snapshot.settings.values.mining_paused = true;
        assert!(!needed(true, &snapshot));
        snapshot.settings.values.mining_paused = false;
        snapshot.login.user_id = None;
        assert!(!needed(true, &snapshot));
    }

    #[tokio::test]
    async fn sleep_request_releases_on_idle_and_shutdown_on_its_original_thread() {
        struct Guard(Arc<Mutex<Vec<std::thread::ThreadId>>>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.lock().unwrap().push(std::thread::current().id());
            }
        }
        let (sender, receiver) = watch::channel(false);
        let (reported, reports) = mpsc::channel();
        let threads = Arc::new(Mutex::new(Vec::new()));
        let acquired = threads.clone();
        let task = tokio::task::spawn_blocking(move || {
            run(
                receiver,
                || {
                    acquired.lock().unwrap().push(std::thread::current().id());
                    Ok::<_, ()>(Guard(acquired.clone()))
                },
                |failed| reported.send(failed).unwrap(),
            );
        });
        let report = || {
            reports
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
        };
        sender.send(true).unwrap();
        assert!(!report());
        sender.send(false).unwrap();
        assert!(!report());
        assert_eq!(threads.lock().unwrap().len(), 2);
        sender.send(true).unwrap();
        assert!(!report());
        drop(sender);
        task.await.unwrap();
        let threads = threads.lock().unwrap();
        assert_eq!(threads.len(), 4);
        assert!(threads.iter().all(|id| *id == threads[0]));
    }

    #[tokio::test]
    async fn failed_sleep_request_is_reported_and_can_retry_after_disabling() {
        let (sender, receiver) = watch::channel(false);
        let (reported, reports) = mpsc::channel();
        let task = tokio::task::spawn_blocking(move || {
            let mut attempts = 0;
            run(
                receiver,
                || {
                    attempts += 1;
                    if attempts == 1 { Err(()) } else { Ok(()) }
                },
                |failed| reported.send(failed).unwrap(),
            );
            assert_eq!(attempts, 2);
        });
        let report = || {
            reports
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
        };
        sender.send(true).unwrap();
        assert!(report());
        sender.send(false).unwrap();
        assert!(!report());
        sender.send(true).unwrap();
        assert!(!report());
        drop(sender);
        task.await.unwrap();
    }
}
