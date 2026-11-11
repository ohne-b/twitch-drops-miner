//! Offline account setup, compiled out of production.
use super::*;
use crate::dto::{AccountProfile, OAuthCode, Snapshot};
use serde_json::json;
use tokio::time::{Duration, Instant};

impl AccountManager {
    pub async fn fixture_profiles(
        self: &Arc<Self>,
        snapshots: Vec<Snapshot>,
    ) -> anyhow::Result<Vec<(Arc<Application>, mpsc::Receiver<CommandRequest>)>> {
        anyhow::ensure!(self.offline.load(std::sync::atomic::Ordering::Acquire));
        anyhow::ensure!(snapshots.len() < 100);
        if let Some(attempt) = self.login.lock().await.take() {
            attempt.cancel.cancel();
        }
        *self.connection.write().await = (
            Connection::from_settings(&*self.primary.settings.read().await),
            random_hex::<16>()?,
        );
        for account in self.overview().await.accounts {
            if account.id != PRIMARY {
                self.request(Request::Remove { id: account.id }).await?;
            }
        }
        let mut registry = self.registry.lock().await.clone();
        registry.accounts.retain(|account| account.id == PRIMARY);
        registry.telegram_account = None;
        let mut workers = vec![];
        for (index, snapshot) in snapshots.into_iter().enumerate() {
            let id = format!("{:032x}", index + 1);
            let data = match self.entries.read().await.get(&id) {
                Some(entry) => entry.app.data.clone(),
                None => Arc::new(DataDirectory::open(
                    self.primary.data.path.join("accounts").join(&id),
                )?),
            };
            data.save_settings(&snapshot.settings.values)?;
            let (app, commands) = Application::open_account(
                data,
                id.clone(),
                self.telegram.clone(),
                Some(&self.connection.read().await.0),
            )?;
            let _ = app.accounts.set(Arc::downgrade(self));
            registry.accounts.push(store::Account {
                id: id.clone(),
                user_id: snapshot.login.user_id,
                label: String::new(),
                enabled: true,
                removed: false,
                unassigned: false,
            });
            *app.settings.write().await = snapshot.settings.values.clone();
            *app.snapshot.write().await = snapshot;
            self.entries.write().await.insert(
                id,
                Entry {
                    app: app.clone(),
                    commands: None,
                    stopped: watch::channel(true).0,
                },
            );
            self.observe(app.clone());
            workers.push((app, commands));
        }
        if let Some(primary) = registry.find_mut(PRIMARY) {
            primary.user_id = self.primary.snapshot.read().await.login.user_id;
            primary.unassigned = false;
        }
        self.save_registry(registry).await?;
        self.publish().await;
        Ok(workers)
    }

    pub(super) async fn fixture_login(
        &self,
        target: Option<String>,
    ) -> Result<serde_json::Value, Error> {
        let mut pending = self.login.lock().await;
        if pending.is_some() {
            return Err(Error::Busy);
        }
        let id = random_hex::<16>().map_err(|_| Error::Storage)?;
        *pending = Some(login::Attempt {
            view: LoginView {
                confirmed: false,
                id: id.clone(),
                target,
                code: Some(OAuthCode {
                    code: "TESTCODE".into(),
                    url: "https://www.twitch.tv/activate".into(),
                }),
                user_id: None,
                name: None,
                avatar_url: None,
                error: None,
            },
            cancel: self.shutdown.child_token(),
            confirmed: Arc::new(tokio::sync::Notify::new()),
            expires: Instant::now() + Duration::from_secs(600),
            session: None,
            profile: None,
        });
        Ok(json!({"attempt":id}))
    }

    pub(super) async fn fixture_confirm(&self, id: &str) -> Result<(), Error> {
        let mut pending = self.login.lock().await;
        let attempt = pending
            .as_mut()
            .filter(|attempt| attempt.view.id == id && !attempt.cancel.is_cancelled())
            .ok_or(Error::Expired)?;
        let registry = self.registry.lock().await;
        let user = attempt
            .view
            .target
            .as_deref()
            .and_then(|id| registry.find(id))
            .and_then(|account| account.user_id)
            .unwrap_or(10000 + registry.accounts.len() as u64);
        let profile = AccountProfile {
            display_name: format!("Fixture {user}"),
            ..Default::default()
        };
        attempt.view.confirmed = true;
        attempt.session = Some(serde_json::from_value(json!({"version":1,"client_id":crate::twitch::CLIENT_ID,"user_id":user,"device_id":"offlinefixture","access_token":"offlinefixture","refresh_token":null})).map_err(|_| Error::Invalid)?);
        attempt.profile = Some(profile.clone());
        attempt.view.code = None;
        attempt.view.user_id = Some(user);
        attempt.view.name = Some(profile.display_name);
        Ok(())
    }

    pub async fn fixture_pending_workers(
        self: &Arc<Self>,
    ) -> Vec<(Arc<Application>, mpsc::Receiver<CommandRequest>)> {
        if !self.offline.load(std::sync::atomic::Ordering::Acquire) {
            return vec![];
        }
        for entry in self.entries.read().await.values() {
            let mut snapshot = entry.app.snapshot.write().await;
            if snapshot.login.user_id.is_none()
                && let Ok(Some(session)) = crate::twitch::oauth::Session::load(&entry.app.data.path)
            {
                snapshot.login.user_id = Some(session.user_id);
                snapshot.login.status = "Logged in".into();
                snapshot.login.oauth_pending = None;
            }
        }
        let workers: Vec<_> = self
            .entries
            .write()
            .await
            .values_mut()
            .filter_map(|entry| {
                entry
                    .commands
                    .take()
                    .map(|commands| (entry.app.clone(), commands))
            })
            .collect();
        for (app, _) in &workers {
            app.snapshot.write().await.login.user_id = self
                .registry
                .lock()
                .await
                .find(&app.account_id)
                .and_then(|account| account.user_id);
            self.observe(app.clone());
        }
        workers
    }
}
