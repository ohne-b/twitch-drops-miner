use super::*;
use crate::{
    app::AppError,
    dto::{AccountProfile, OAuthCode},
    twitch::{
        TwitchClient, TwitchError, TwitchHttp,
        oauth::{DeviceLogin, Session},
    },
};
use serde_json::{Value, json};
use tokio::{
    sync::Notify,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, PartialEq)]
pub struct LoginView {
    pub id: String,
    pub target: Option<String>,
    pub code: Option<OAuthCode>,
    pub confirmed: bool,
    pub user_id: Option<u64>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub error: Option<String>,
}

pub(super) struct Attempt {
    pub view: LoginView,
    pub cancel: CancellationToken,
    pub confirmed: Arc<Notify>,
    pub(super) expires: Instant,
    pub(super) session: Option<Session>,
    pub(super) profile: Option<AccountProfile>,
}

impl AccountManager {
    pub(super) async fn invalidate_login(&self, target: Option<&str>, reason: &str) {
        let mut pending = self.login.lock().await;
        if let Some(attempt) = pending
            .as_mut()
            .filter(|a| target.is_none() || a.view.target.as_deref() == target)
        {
            attempt.cancel.cancel();
            attempt.session = None;
            attempt.profile = None;
            attempt.view.code = None;
            attempt.view.user_id = None;
            attempt.view.name = None;
            attempt.view.avatar_url = None;
            attempt.view.error = Some(reason.into());
        }
    }
    pub(super) async fn start_login(
        self: &Arc<Self>,
        target: Option<String>,
        associate_existing: bool,
    ) -> Result<Value, Error> {
        if let Some(id) = &target {
            self.account(id).await?;
            let registry = self.registry.lock().await;
            if registry.find(id).is_some_and(|a| a.unassigned) && !associate_existing {
                return Err(Error::Identity);
            }
        }
        #[cfg(feature = "dashboard-fixture")]
        if self.offline.load(std::sync::atomic::Ordering::Acquire) {
            return self.fixture_login(target).await;
        }
        let mut pending = self.login.lock().await;
        if pending
            .as_ref()
            .is_some_and(|a| a.expires > Instant::now() && a.view.error.is_none())
        {
            return Err(Error::Busy);
        }
        if let Some(old) = pending.take() {
            old.cancel.cancel();
        }
        let id = random_hex::<16>().map_err(|_| Error::Storage)?;
        let cancel = self.shutdown.child_token();
        let confirmed = Arc::new(Notify::new());
        let expires = Instant::now() + Duration::from_secs(600);
        *pending = Some(Attempt {
            view: LoginView {
                confirmed: false,
                id: id.clone(),
                target,
                code: None,
                user_id: None,
                name: None,
                avatar_url: None,
                error: None,
            },
            cancel: cancel.clone(),
            confirmed: confirmed.clone(),
            expires,
            session: None,
            profile: None,
        });
        drop(pending);
        let owned = self.clone();
        let attempt_id = id.clone();
        self.tasks.spawn(async move {
            let result = tokio::time::timeout_at(expires, owned.authorize(&attempt_id, cancel.clone(), confirmed)).await.unwrap_or(Err(TwitchError::Expired));
            {
                let mut pending = owned.login.lock().await;
                if let Some(attempt) = pending.as_mut().filter(|a| a.view.id == attempt_id && !a.cancel.is_cancelled()) {
                    match result {
                        Ok((session, profile)) => {
                            attempt.view.user_id = Some(session.user_id);
                            attempt.view.name = profile.as_ref().map(|p| p.display_name.clone());
                            attempt.view.avatar_url = profile.as_ref().and_then(|p| p.avatar_url.clone());
                            attempt.view.code = None;
                            attempt.session = Some(session);
                            attempt.profile = profile;
                        },
                        Err(error) => {
                            attempt.view.code = None;
                            attempt.view.error = Some(match error {
                                TwitchError::Expired => "account_login_expired",
                                TwitchError::Denied => "account_login_denied",
                                TwitchError::Cancelled => "account_login_cancelled",
                                _ => "request_failed",
                            }.into());
                        },
                    }
                }
            }
            owned.publish().await;
            tokio::select! { _ = cancel.cancelled() => {}, _ = tokio::time::sleep_until(expires) => {
                let mut pending = owned.login.lock().await;
                if let Some(attempt) = pending.as_mut().filter(|a| a.view.id == attempt_id) {
                    attempt.session = None;
                    attempt.view.code = None;
                    attempt.view.error = Some("account_login_expired".into());
                }
            } }
            owned.publish().await;
        });
        Ok(json!({"attempt":id}))
    }

    async fn authorize(
        &self,
        id: &str,
        cancel: CancellationToken,
        confirmed: Arc<Notify>,
    ) -> Result<(Session, Option<AccountProfile>), TwitchError> {
        let mut settings = Settings::default();
        self.connection.read().await.0.apply(&mut settings);
        let mut http = TwitchHttp::new(&settings, None, cancel)?;
        http.shared = Some(self.network.clone());
        http.discover_device().await?;
        let http = Arc::new(http);
        let login = DeviceLogin::start(http.clone()).await?;
        {
            let mut pending = self.login.lock().await;
            let attempt = pending
                .as_mut()
                .filter(|a| a.view.id == id && !a.cancel.is_cancelled())
                .ok_or(TwitchError::Cancelled)?;
            attempt.view.code = Some(login.code.clone());
        }
        self.publish().await;
        let session = login.finish(&confirmed).await?;
        let profile = TwitchClient::new(http, &session).account_profile().await?;
        Ok((session, profile))
    }

    pub(super) async fn finish_login(
        self: &Arc<Self>,
        id: &str,
        label: String,
        copy_from: Option<Target>,
        copy_auto_claim: bool,
    ) -> Result<Value, AppError> {
        if !store::valid_label(&label) {
            return Err(Error::Invalid.into());
        }
        let (session, profile, target) = {
            let pending = self.login.lock().await;
            let attempt = pending
                .as_ref()
                .filter(|a| {
                    a.view.id == id && a.expires > Instant::now() && !a.cancel.is_cancelled()
                })
                .ok_or(Error::Expired)?;
            (
                attempt.session.clone().ok_or(Error::Required)?,
                attempt.profile.clone(),
                attempt.view.target.clone(),
            )
        };
        let mut registry = self.registry.lock().await.clone();
        if let Some(target) = &target {
            let account = registry.find(target).ok_or(Error::NotFound)?;
            if account.removed {
                return Err(Error::NotFound.into());
            }
            if account.user_id.is_some_and(|user| user != session.user_id) {
                return Err(Error::Identity.into());
            }
        }
        let existing = registry
            .accounts
            .iter()
            .find(|a| a.user_id == Some(session.user_id))
            .cloned();
        if let Some(existing) = &existing {
            if target.as_ref().is_some_and(|target| target != &existing.id) {
                return Err(Error::Identity.into());
            }
            if !existing.removed && target.is_none() {
                if let Some(attempt) = self.login.lock().await.take() {
                    attempt.cancel.cancel();
                }
                return Ok(json!({"id":existing.id,"existing":true}));
            }
        }
        let account_id = existing
            .as_ref()
            .map(|a| a.id.clone())
            .or(target)
            .or_else(|| {
                registry
                    .find(PRIMARY)
                    .filter(|a| a.user_id.is_none() && !a.unassigned)
                    .map(|a| a.id.clone())
            })
            .unwrap_or(random_hex::<16>().map_err(|_| Error::Storage)?);
        let old = self
            .entries
            .read()
            .await
            .get(&account_id)
            .map(|e| e.app.clone());
        let _settings_permit = if let Some(app) = &old {
            Some(
                app.settings_slot
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| AppError::Unavailable)?,
            )
        } else {
            None
        };
        let data = if let Some(old) = &old {
            old.data.clone()
        } else {
            Arc::new(
                DataDirectory::open(self.primary.data.path.join("accounts").join(&account_id))
                    .map_err(|_| Error::Storage)?,
            )
        };
        let mut settings = data.settings().map_err(|_| Error::Storage)?;
        if let Some(source) = copy_from {
            let source = self.preference_source(&source).await?;
            settings = settings
                .patched(&api::preferences(&source, copy_auto_claim))
                .map_err(|_| AppError::InvalidSettings)?;
        }
        self.connection.read().await.0.apply(&mut settings);
        let account = store::Account {
            id: account_id.clone(),
            user_id: Some(session.user_id),
            label: label.trim().into(),
            enabled: true,
            removed: false,
            unassigned: false,
        };
        if let Some(saved) = registry.find_mut(&account_id) {
            *saved = account;
        } else {
            registry.accounts.push(account);
        }
        if let Some(app) = &old
            && !app.shutdown.is_cancelled()
        {
            app.command(crate::app::Command::Logout).await?;
        }
        // Identity is durable before credentials, so a failed save cannot attach them to another account.
        self.save_registry(registry).await?;
        let saved_data = data.clone();
        tokio::task::spawn_blocking(move || {
            saved_data
                .save_settings(&settings)
                .map_err(|_| Error::Storage)?;
            session.save(&saved_data.path).map_err(|_| Error::Storage)
        })
        .await
        .map_err(|_| Error::Storage)??;
        if let Some(old) = old.filter(|a| !a.shutdown.is_cancelled()) {
            let settings = old.data.settings().map_err(|_| Error::Storage)?;
            *old.settings.write().await = settings.clone();
            {
                let mut snapshot = old.snapshot.write().await;
                snapshot.settings.values = settings;
                snapshot.settings.revision = random_hex::<16>().map_err(|_| Error::Storage)?;
                snapshot.settings.refresh_game_keys();
                snapshot.login.profile = profile;
            }
            old.session_changed.send_modify(|value| *value += 1);
        } else {
            let connection = self.connection.read().await.0.clone();
            let (app, commands) = Application::open_account(
                data,
                account_id.clone(),
                self.telegram.clone(),
                Some(&connection),
            )
            .map_err(|_| Error::Storage)?;
            let _ = app.accounts.set(Arc::downgrade(self));
            app.snapshot.write().await.login.profile = profile;
            let mut entry = Entry {
                app,
                commands: Some(commands),
                stopped: watch::channel(true).0,
            };
            if self.started.load(std::sync::atomic::Ordering::Acquire) {
                self.start_entry(&mut entry, Duration::ZERO);
            }
            self.entries.write().await.insert(account_id.clone(), entry);
        }
        if let Some(attempt) = self.login.lock().await.take() {
            attempt.cancel.cancel();
        }
        self.failures.write().await.remove(&account_id);
        Ok(json!({"id":account_id,"existing":false}))
    }
}
