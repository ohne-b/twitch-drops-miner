mod api;
#[cfg(feature = "dashboard-fixture")]
mod fixture;
mod login;
mod store;
#[cfg(test)]
mod tests;

pub use api::{Request, Target};
pub use login::LoginView;
pub use store::{Connection, MAX_ACCOUNTS, PRIMARY, valid_id};

use crate::{
    app::{Application, CommandRequest, Notification},
    config::Settings,
    dto::MiningState,
    notifications::Telegram,
    random_hex,
    runtime::Runtime,
    store::DataDirectory,
};
use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use store::Registry;
use tokio::sync::{Mutex, RwLock, Semaphore, broadcast, mpsc, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum Error {
    #[error("account_not_found")]
    NotFound,
    #[error("account_required")]
    Required,
    #[error("account_limit")]
    Limit,
    #[error("account_busy")]
    Busy,
    #[error("account_identity_mismatch")]
    Identity,
    #[error("account_login_expired")]
    Expired,
    #[error("account_storage")]
    Storage,
    #[error("invalid_request")]
    Invalid,
    #[error("settings_conflict")]
    Conflict,
    #[error("shutting_down")]
    ShuttingDown,
}

impl Error {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotFound => "account_not_found",
            Self::Required => "account_required",
            Self::Limit => "account_limit",
            Self::Busy => "account_busy",
            Self::Identity => "account_identity_mismatch",
            Self::Expired => "account_login_expired",
            Self::Storage => "account_storage",
            Self::Invalid => "invalid_request",
            Self::Conflict => "settings_conflict",
            Self::ShuttingDown => "shutting_down",
        }
    }
}

#[derive(Clone, Serialize, PartialEq)]
pub struct Summary {
    pub id: String,
    pub instance: Option<String>,
    pub label: String,
    pub user_id: Option<u64>,
    pub name: String,
    pub avatar_url: Option<String>,
    pub connected: bool,
    pub unassigned: bool,
    pub unavailable: bool,
    pub paused: bool,
    pub mining: MiningState,
    pub game: Option<String>,
    pub reward: Option<String>,
    pub settings_revision: String,
}

#[derive(Clone, Serialize)]
pub struct Overview {
    pub instance: String,
    pub revision: u64,
    pub accounts: Vec<Summary>,
    pub limit: usize,
    pub login: Option<LoginView>,
}

struct Entry {
    app: Arc<Application>,
    commands: Option<mpsc::Receiver<CommandRequest>>,
    stopped: watch::Sender<bool>,
}

/// Owns account runtimes; selecting a view never changes which runtimes are active.
pub struct AccountManager {
    pub primary: Arc<Application>,
    pub shutdown: CancellationToken,
    pub notifications: broadcast::Sender<Notification>,
    pub(crate) telegram: Arc<Telegram>,
    pub(crate) network: Arc<crate::twitch::shared::SharedNetwork>,
    entries: RwLock<BTreeMap<String, Entry>>,
    failures: RwLock<std::collections::HashSet<String>>,
    registry: Mutex<Registry>,
    connection: RwLock<(Connection, String)>,
    overview: RwLock<Overview>,
    publication: Mutex<()>,
    changed: watch::Sender<u64>,
    operations: Arc<Semaphore>,
    tasks: TaskTracker,
    writes: TaskTracker,
    started: std::sync::atomic::AtomicBool,
    #[cfg(feature = "dashboard-fixture")]
    offline: std::sync::atomic::AtomicBool,
    login: Mutex<Option<login::Attempt>>,
}

impl AccountManager {
    pub fn open(directory: PathBuf) -> anyhow::Result<(Arc<Self>, mpsc::Receiver<CommandRequest>)> {
        let data = Arc::new(DataDirectory::open(directory)?);
        let registry = Registry::load(&data.path)?;
        let legacy = data.settings();
        // Without the shared file, unreadable legacy proxy settings cannot safely become defaults.
        let (settings, primary_failed) = match legacy {
            Ok(settings) => (settings, false),
            Err(error) if !data.path.join("connection.json").exists() => return Err(error),
            Err(_) => (Settings::default(), true),
        };
        let connection = Connection::load(&data.path, &settings)?;
        let telegram = Arc::new(Telegram::open(&data.path)?);
        let (primary, commands) = Application::with_account_settings(
            data.clone(),
            PRIMARY.into(),
            telegram.clone(),
            Some(&connection),
            settings,
        )?;
        let mut entries = BTreeMap::new();
        let mut failures = std::collections::HashSet::new();
        if primary_failed {
            primary.shutdown.cancel();
            failures.insert(PRIMARY.to_owned());
        }
        entries.insert(
            PRIMARY.into(),
            Entry {
                app: primary.clone(),
                commands: None,
                stopped: watch::channel(true).0,
            },
        );
        for account in registry
            .accounts
            .iter()
            .filter(|a| a.id != PRIMARY && !a.removed)
        {
            let opened = (|| {
                let data = Arc::new(DataDirectory::open(
                    data.path.join("accounts").join(&account.id),
                )?);
                Application::open_account(
                    data,
                    account.id.clone(),
                    telegram.clone(),
                    Some(&connection),
                )
            })();
            match opened {
                Ok((app, commands)) => {
                    entries.insert(
                        account.id.clone(),
                        Entry {
                            app,
                            commands: Some(commands),
                            stopped: watch::channel(true).0,
                        },
                    );
                }
                Err(_) => {
                    failures.insert(account.id.clone());
                    tracing::warn!(
                        account = account.id,
                        "Cannot open account storage; other accounts remain available"
                    );
                }
            }
        }
        let shutdown = CancellationToken::new();
        let network = crate::twitch::shared::SharedNetwork::new(
            shutdown.child_token(),
            connection.catalog_api_key.clone(),
        );
        let manager = Arc::new(Self {
            primary,
            shutdown,
            network,
            notifications: broadcast::channel(64).0,
            telegram,
            entries: RwLock::new(entries),
            failures: RwLock::new(failures),
            registry: Mutex::new(registry),
            connection: RwLock::new((connection, random_hex::<16>()?)),
            overview: RwLock::new(Overview {
                instance: random_hex::<16>()?,
                revision: 1,
                accounts: vec![],
                limit: MAX_ACCOUNTS,
                login: None,
            }),
            publication: Mutex::new(()),
            changed: watch::channel(1).0,
            operations: Arc::new(Semaphore::new(1)),
            tasks: TaskTracker::new(),
            writes: TaskTracker::new(),
            started: std::sync::atomic::AtomicBool::new(false),
            login: Mutex::new(None),
            #[cfg(feature = "dashboard-fixture")]
            offline: std::sync::atomic::AtomicBool::new(false),
        });
        for entry in manager
            .entries
            .try_read()
            .expect("new account registry")
            .values()
        {
            let _ = entry.app.accounts.set(Arc::downgrade(&manager));
        }
        Ok((manager, commands))
    }

    pub async fn start(self: &Arc<Self>, commands: mpsc::Receiver<CommandRequest>) {
        if self.started.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        let mut entries = self.entries.write().await;
        if !self.primary.shutdown.is_cancelled() {
            entries.get_mut(PRIMARY).expect("original account").commands = Some(commands);
        }
        for (index, entry) in entries.values_mut().enumerate() {
            self.start_entry(entry, std::time::Duration::from_millis(index as u64 * 750));
        }
        drop(entries);
        let owned = self.clone();
        self.tasks.spawn(async move {
            crate::notifications::run(owned.primary.clone(), owned.notifications.subscribe()).await;
        });
        self.publish().await;
    }

    #[cfg(feature = "dashboard-fixture")]
    pub async fn fixture_observe(self: &Arc<Self>) {
        self.offline
            .store(true, std::sync::atomic::Ordering::Release);
        self.telegram.disable_for_fixture();
        self.observe(self.primary.clone());
        self.publish().await;
    }

    #[cfg(feature = "dashboard-fixture")]
    pub fn fixture_worker(&self, worker: tokio::task::JoinHandle<()>) {
        self.tasks.spawn(async move {
            let _ = worker.await;
        });
    }

    pub async fn wait_overview(
        &self,
        instance: Option<&str>,
        revision: Option<u64>,
        cancel: &CancellationToken,
    ) -> Result<Overview, Error> {
        let mut changes = self.subscribe();
        let current = self.overview().await;
        if instance != Some(current.instance.as_str()) || revision != Some(current.revision) {
            return Ok(current);
        }
        tokio::select! { biased;
            _ = self.shutdown.cancelled() => return Err(Error::ShuttingDown),
            _ = cancel.cancelled() => return Err(Error::ShuttingDown),
            _ = tokio::time::sleep(std::time::Duration::from_secs(20)) => {},
            _ = changes.changed() => {},
        }
        Ok(self.overview().await)
    }

    fn start_entry(self: &Arc<Self>, entry: &mut Entry, delay: std::time::Duration) {
        let Some(commands) = entry.commands.take() else {
            return;
        };
        entry.stopped.send_replace(false);
        let stopped = entry.stopped.clone();
        let app = entry.app.clone();
        let manager = self.clone();
        self.observe(app.clone());
        self.tasks.spawn(async move {
            tokio::select! {
                _ = manager.shutdown.cancelled() => app.shutdown.cancel(),
                _ = tokio::time::sleep(delay) => {},
            }
            let mut runtime = Runtime::start(app.clone(), commands);
            let result = tokio::select! {
                result = runtime.wait() => result,
                _ = manager.shutdown.cancelled() => runtime.shutdown().await,
            };
            app.drain_writes().await;
            if result.is_err() {
                app.activity("gui.backend.worker_failed", &[]).await;
            }
            stopped.send_replace(true);
            manager.publish().await;
        });
    }

    fn observe(self: &Arc<Self>, app: Arc<Application>) {
        let mut snapshots = app.snapshot.subscribe();
        let mut notifications = app.notifications.subscribe();
        let manager = self.clone();
        self.tasks.spawn(async move {
            loop {
                tokio::select! { biased;
                    _ = manager.shutdown.cancelled() => break,
                    _ = app.shutdown.cancelled() => break,
                    result = snapshots.changed() => {
                        if result.is_err() { break; }
                        manager.publish().await;
                    },
                    event = notifications.recv() => if let Ok(mut event) = event {
                        let overview = manager.overview.read().await;
                        if overview.accounts.len() > 1
                            && let Some(account) = overview.accounts.iter().find(|a| a.id == app.account_id) {
                            event.title = format!("{}: {}", account.name, event.title);
                        }
                        let _ = manager.notifications.send(event);
                    },
                }
            }
        });
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
    pub(crate) fn global_writes(&self) -> TaskTracker {
        self.writes.clone()
    }
    pub(crate) async fn proxy(&self) -> String {
        self.connection.read().await.0.proxy.clone()
    }
    pub(crate) async fn account_for_user(&self, user: u64) -> Result<Arc<Application>, Error> {
        let id = self
            .registry
            .lock()
            .await
            .accounts
            .iter()
            .find(|a| a.user_id == Some(user) && !a.removed)
            .map(|a| a.id.clone())
            .ok_or(Error::NotFound)?;
        self.account(&id).await
    }

    pub async fn overview(&self) -> Overview {
        self.publish().await;
        self.overview.read().await.clone()
    }

    async fn publish(&self) {
        let _publication = self.publication.lock().await;
        let registry = self.registry.lock().await.clone();
        let apps: BTreeMap<_, _> = self
            .entries
            .read()
            .await
            .iter()
            .map(|(id, e)| (id.clone(), e.app.clone()))
            .collect();
        let mut accounts = vec![];
        for account in registry.accounts.iter().filter(|a| !a.removed) {
            let Some(app) = apps.get(&account.id) else {
                accounts.push(Summary {
                    id: account.id.clone(),
                    instance: None,
                    label: account.label.clone(),
                    user_id: account.user_id,
                    name: if account.label.is_empty() {
                        "Twitch account".into()
                    } else {
                        account.label.clone()
                    },
                    avatar_url: None,
                    connected: false,
                    unassigned: account.unassigned,
                    unavailable: true,
                    paused: false,
                    mining: MiningState::AccountRequired,
                    game: None,
                    reward: None,
                    settings_revision: String::new(),
                });
                continue;
            };
            let snapshot = app.snapshot.read().await;
            let profile = snapshot.login.profile.as_ref();
            let name = if !account.label.is_empty() {
                account.label.clone()
            } else if let Some(profile) = profile {
                profile.display_name.clone()
            } else {
                account
                    .user_id
                    .map_or_else(|| "Twitch account".into(), |id| format!("Twitch {id}"))
            };
            accounts.push(Summary {
                id: account.id.clone(),
                instance: Some(snapshot.instance.clone()),
                label: account.label.clone(),
                user_id: account.user_id,
                name,
                avatar_url: profile.and_then(|p| p.avatar_url.clone()),
                connected: snapshot.login.user_id.is_some(),
                unassigned: account.unassigned,
                unavailable: app.shutdown.is_cancelled(),
                paused: snapshot.settings.values.mining_paused,
                mining: snapshot.mining.state,
                game: snapshot.current_drop.as_ref().map(|d| d.game_name.clone()),
                reward: snapshot.current_drop.as_ref().map(|d| d.drop_name.clone()),
                settings_revision: snapshot.settings.revision.clone(),
            });
        }
        let login = self.login.lock().await.as_ref().map(|a| a.view.clone());
        let mut overview = self.overview.write().await;
        if overview.accounts != accounts || overview.login != login {
            overview.accounts = accounts;
            overview.login = login;
            overview.revision += 1;
            self.changed.send_replace(overview.revision);
        }
    }

    pub async fn account(&self, id: &str) -> Result<Arc<Application>, Error> {
        if self.shutdown.is_cancelled() {
            return Err(Error::ShuttingDown);
        }
        if !valid_id(id)
            || !self
                .registry
                .lock()
                .await
                .find(id)
                .is_some_and(|a| !a.removed)
        {
            return Err(Error::NotFound);
        }
        if self.failures.read().await.contains(id) {
            return Err(Error::Storage);
        }
        self.entries
            .read()
            .await
            .get(id)
            .map(|e| e.app.clone())
            .ok_or(Error::NotFound)
    }

    pub async fn sole_account(&self) -> Result<Arc<Application>, Error> {
        let registry = self.registry.lock().await;
        let mut accounts = registry.accounts.iter().filter(|a| !a.removed);
        let id = accounts.next().ok_or(Error::Required)?.id.clone();
        if accounts.next().is_some() {
            return Err(Error::Required);
        }
        drop(registry);
        self.account(&id).await
    }

    pub(crate) async fn identity_matches(&self, id: &str, user_id: u64) -> bool {
        self.registry
            .lock()
            .await
            .find(id)
            .is_some_and(|a| !a.removed && !a.unassigned && a.user_id == Some(user_id))
    }

    async fn save_registry(&self, next: Registry) -> Result<(), Error> {
        let path = self.primary.data.path.clone();
        let saved = next.clone();
        tokio::task::spawn_blocking(move || saved.save(&path))
            .await
            .map_err(|_| Error::Storage)?
            .map_err(|_| Error::Storage)?;
        *self.registry.lock().await = next;
        Ok(())
    }

    pub async fn connection(&self) -> serde_json::Value {
        let (values, revision) = &*self.connection.read().await;
        serde_json::json!({"values": {
            "proxy": values.proxy,
            "connection_quality": values.connection_quality,
            "minimum_refresh_interval_minutes": values.minimum_refresh_interval_minutes,
        }, "catalog_api_key_configured": values.catalog_api_key.is_some(), "revision":revision})
    }

    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        for entry in self.entries.read().await.values() {
            entry.app.shutdown.cancel();
        }
        // Close acceptance before the final drain, including writes already waiting for their turn.
        let _permit = self.operations.acquire().await;
        for entry in self.entries.read().await.values() {
            entry.app.shutdown.cancel();
        }
        if let Some(attempt) = self.login.lock().await.as_ref() {
            attempt.cancel.cancel();
        }
        self.writes.close();
        self.writes.wait().await;
        self.tasks.close();
        self.tasks.wait().await;
        self.network.shutdown().await;
    }

    pub(crate) async fn telegram_account(&self) -> Result<Arc<Application>, Error> {
        let registry = self.registry.lock().await;
        let selected = registry.telegram_account.clone();
        drop(registry);
        match selected {
            Some(id) => self.account(&id).await,
            None => self.sole_account().await,
        }
    }
}

impl Drop for AccountManager {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}
