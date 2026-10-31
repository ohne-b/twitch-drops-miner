use super::*;
use crate::app::{AppError, Command};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub id: String,
    pub revision: String,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    StartLogin {
        target: Option<String>,
        #[serde(default)]
        associate_existing: bool,
    },
    ConfirmLogin {
        attempt: String,
    },
    CancelLogin {
        attempt: String,
    },
    FinishLogin {
        attempt: String,
        label: String,
        copy_from: Option<Target>,
        #[serde(default)]
        copy_auto_claim: bool,
    },
    Rename {
        id: String,
        label: String,
    },
    SignOut {
        id: String,
    },
    Remove {
        id: String,
    },
    Retry {
        id: String,
    },
    Pause {
        targets: Vec<Target>,
        paused: bool,
    },
    CopyPreferences {
        source: Target,
        targets: Vec<Target>,
        #[serde(default)]
        auto_claim: bool,
    },
    Connection {
        revision: String,
        values: Connection,
    },
    TelegramAccount {
        id: String,
    },
}

impl AccountManager {
    pub async fn request(self: &Arc<Self>, request: Request) -> Result<Value, AppError> {
        let permit = self
            .operations
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::ShuttingDown)?;
        if self.shutdown.is_cancelled() {
            return Err(Error::ShuttingDown.into());
        }
        let manager = self.clone();
        self.writes
            .spawn(async move {
                let _permit = permit;
                if manager.shutdown.is_cancelled() {
                    return Err(Error::ShuttingDown.into());
                }
                let result = manager.execute(request).await;
                manager.publish().await;
                result
            })
            .await
            .map_err(|_| AppError::Unavailable)?
    }

    async fn execute(self: &Arc<Self>, request: Request) -> Result<Value, AppError> {
        match request {
            Request::StartLogin {
                target,
                associate_existing,
            } => self
                .start_login(target, associate_existing)
                .await
                .map_err(Into::into),
            Request::ConfirmLogin { attempt } => {
                #[cfg(feature = "dashboard-fixture")]
                if self.offline.load(std::sync::atomic::Ordering::Acquire) {
                    self.fixture_confirm(&attempt).await?;
                    return Ok(json!({}));
                }
                let mut login = self.login.lock().await;
                let login = login
                    .as_mut()
                    .filter(|a| {
                        a.view.id == attempt
                            && !a.cancel.is_cancelled()
                            && a.expires > tokio::time::Instant::now()
                    })
                    .ok_or(Error::Expired)?;
                if !login.view.confirmed {
                    login.view.confirmed = true;
                    login.confirmed.notify_one();
                }
                Ok(json!({}))
            }
            Request::CancelLogin { attempt } => {
                let mut login = self.login.lock().await;
                if login.as_ref().is_some_and(|a| a.view.id == attempt)
                    && let Some(attempt) = login.take()
                {
                    attempt.cancel.cancel();
                }
                Ok(json!({}))
            }
            Request::FinishLogin {
                attempt,
                label,
                copy_from,
                copy_auto_claim,
            } => {
                self.finish_login(&attempt, label, copy_from, copy_auto_claim)
                    .await
            }
            Request::Rename { id, label } => {
                self.account(&id).await?;
                if !store::valid_label(&label) {
                    return Err(Error::Invalid.into());
                }
                let mut next = self.registry.lock().await.clone();
                next.find_mut(&id).ok_or(Error::NotFound)?.label = label.trim().into();
                self.save_registry(next).await?;
                Ok(json!({}))
            }
            Request::SignOut { id } => {
                let app = self.account(&id).await?;
                self.invalidate_login(Some(&id), "account_login_cancelled")
                    .await;
                app.command(Command::Logout).await?;
                Ok(json!({}))
            }
            Request::Remove { id } => {
                if !valid_id(&id)
                    || !self
                        .registry
                        .lock()
                        .await
                        .find(&id)
                        .is_some_and(|a| !a.removed)
                {
                    return Err(Error::NotFound.into());
                }
                self.invalidate_login(Some(&id), "account_login_cancelled")
                    .await;
                let app = self
                    .entries
                    .read()
                    .await
                    .get(&id)
                    .map(|entry| entry.app.clone());
                // Logout drains the generation before deleting credentials, even on disconnect.
                if let Some(app) = &app
                    && !app.shutdown.is_cancelled()
                {
                    app.command(Command::Logout).await?;
                } else {
                    if let Some(mut stopped) = self
                        .entries
                        .read()
                        .await
                        .get(&id)
                        .map(|entry| entry.stopped.subscribe())
                    {
                        while !*stopped.borrow_and_update() {
                            if stopped.changed().await.is_err() {
                                break;
                            }
                        }
                    }
                    let data = match &app {
                        Some(app) => app.data.clone(),
                        None => Arc::new(
                            DataDirectory::open(self.primary.data.path.join("accounts").join(&id))
                                .map_err(|_| Error::Storage)?,
                        ),
                    };
                    tokio::task::spawn_blocking(move || {
                        crate::twitch::oauth::Session::remove(&data.path)
                    })
                    .await
                    .map_err(|_| Error::Storage)?
                    .map_err(|_| Error::Storage)?;
                }
                let mut next = self.registry.lock().await.clone();
                let account = next.find_mut(&id).ok_or(Error::NotFound)?;
                account.removed = true;
                account.enabled = false;
                if next.telegram_account.as_deref() == Some(&id) {
                    next.telegram_account = None;
                }
                self.save_registry(next).await?;
                if let Some(app) = app {
                    app.shutdown.cancel();
                    let mut stopped = self
                        .entries
                        .read()
                        .await
                        .get(&id)
                        .ok_or(Error::NotFound)?
                        .stopped
                        .subscribe();
                    while !*stopped.borrow_and_update() {
                        if stopped.changed().await.is_err() {
                            break;
                        }
                    }
                }
                // Keep the storage lease until outstanding readers release their old application.
                Ok(json!({}))
            }
            Request::Retry { id } => {
                if !valid_id(&id)
                    || !self
                        .registry
                        .lock()
                        .await
                        .find(&id)
                        .is_some_and(|a| !a.removed)
                {
                    return Err(Error::NotFound.into());
                }
                let data = {
                    let entries = self.entries.read().await;
                    if let Some(entry) = entries.get(&id) {
                        if !*entry.stopped.borrow() || !entry.app.shutdown.is_cancelled() {
                            return Err(Error::Busy.into());
                        }
                        entry.app.data.clone()
                    } else {
                        Arc::new(
                            DataDirectory::open(self.primary.data.path.join("accounts").join(&id))
                                .map_err(|_| Error::Storage)?,
                        )
                    }
                };
                let connection = self.connection.read().await.0.clone();
                let (app, commands) = Application::open_account(
                    data,
                    id.clone(),
                    self.telegram.clone(),
                    Some(&connection),
                )
                .map_err(|_| Error::Storage)?;
                let _ = app.accounts.set(Arc::downgrade(self));
                let mut entry = Entry {
                    app,
                    commands: Some(commands),
                    stopped: watch::channel(true).0,
                };
                if self.started.load(std::sync::atomic::Ordering::Acquire) {
                    self.start_entry(&mut entry, std::time::Duration::ZERO);
                }
                self.entries.write().await.insert(id.clone(), entry);
                self.failures.write().await.remove(&id);
                Ok(json!({}))
            }
            Request::Pause { targets, paused } => {
                self.apply_targets(targets, json!({"mining_paused":paused}))
                    .await
            }
            Request::CopyPreferences {
                source,
                targets,
                auto_claim,
            } => {
                let source = self.preference_source(&source).await?;
                self.apply_targets(targets, preferences(&source, auto_claim))
                    .await
            }
            Request::Connection {
                revision,
                mut values,
            } => {
                if !values.valid() {
                    return Err(AppError::InvalidSettings);
                }
                if self.connection.read().await.1 != revision {
                    return Err(Error::Conflict.into());
                }
                // Older clients omit the key: retain it. An explicit empty value clears it.
                if values.catalog_api_key.is_none() {
                    values
                        .catalog_api_key
                        .clone_from(&self.connection.read().await.0.catalog_api_key);
                }
                values.catalog_api_key = values.catalog_api_key.filter(|key| !key.is_empty());
                let next_revision = random_hex::<16>().map_err(|_| Error::Storage)?;
                let path = self.primary.data.path.clone();
                let saved = values.clone();
                tokio::task::spawn_blocking(move || saved.save(&path))
                    .await
                    .map_err(|_| Error::Storage)?
                    .map_err(|_| Error::Storage)?;
                *self.connection.write().await = (values.clone(), next_revision);
                self.invalidate_login(None, "account_login_connection_changed")
                    .await;
                self.network
                    .configure_catalog(values.catalog_api_key.clone())
                    .await;
                let apps: Vec<_> = self
                    .entries
                    .read()
                    .await
                    .values()
                    .map(|e| e.app.clone())
                    .collect();
                let mut unavailable = vec![];
                for app in apps {
                    let _permit = app
                        .settings_slot
                        .acquire()
                        .await
                        .map_err(|_| AppError::Unavailable)?;
                    values.apply(&mut *app.settings.write().await);
                    {
                        let mut snapshot = app.snapshot.write().await;
                        values.apply(&mut snapshot.settings.values);
                        snapshot.settings.revision =
                            random_hex::<16>().map_err(|_| Error::Storage)?;
                    }
                    drop(_permit);
                    if !app.shutdown.is_cancelled()
                        && app.command(Command::SettingsChanged).await.is_err()
                    {
                        unavailable.push(app.account_id.clone());
                    }
                }
                self.telegram.cancel_pending();
                let mut response = self.connection().await;
                response["unavailable"] = json!(unavailable);
                Ok(response)
            }
            Request::TelegramAccount { id } => {
                self.account(&id).await?;
                let mut next = self.registry.lock().await.clone();
                next.telegram_account = Some(id);
                self.save_registry(next).await?;
                Ok(json!({}))
            }
        }
    }

    pub(super) async fn preference_source(&self, target: &Target) -> Result<Settings, AppError> {
        let app = self.account(&target.id).await?;
        let snapshot = app.snapshot.read().await;
        if snapshot.settings.revision != target.revision {
            return Err(Error::Conflict.into());
        }
        Ok(snapshot.settings.values.clone())
    }

    async fn apply_targets(&self, targets: Vec<Target>, patch: Value) -> Result<Value, AppError> {
        let mut unique = std::collections::HashSet::new();
        if targets.is_empty()
            || targets.len() > MAX_ACCOUNTS
            || targets
                .iter()
                .any(|t| !valid_id(&t.id) || !unique.insert(&t.id) || t.revision.len() > 64)
        {
            return Err(Error::Invalid.into());
        }
        let mut results = vec![];
        for target in targets {
            let result = async {
                let app = self.account(&target.id).await?;
                let mut patch = patch.clone();
                patch["revision"] = json!(target.revision);
                app.update_settings(patch).await
            }
            .await;
            results.push(match result {
                Ok(settings) => json!({"id":target.id,"ok":true,"revision":settings.revision}),
                Err(error) => json!({"id":target.id,"ok":false,"error":error.to_string()}),
            });
        }
        Ok(json!({"results":results}))
    }
}

pub(super) fn preferences(source: &Settings, auto_claim: bool) -> Value {
    let mut result = json!({
        "games_to_watch":source.games_to_watch,"game_metadata":source.game_metadata,
        "mining_priority_mode":source.mining_priority_mode,"auto_mine_badges":source.auto_mine_badges,
        "auto_mine_emotes":source.auto_mine_emotes,"drop_name_blacklist":source.drop_name_blacklist,
        "mining_benefits":source.mining_benefits,
    });
    if auto_claim {
        result["auto_claim_drops"] = json!(source.auto_claim_drops);
    }
    result
}
