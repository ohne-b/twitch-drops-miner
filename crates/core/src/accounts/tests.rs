use super::*;
use crate::{
    app::{AppError, Command},
    store::atomic_json,
    twitch::oauth::Session,
};
use serde_json::json;
use tokio::time::{Duration, Instant};

fn session(user: u64) -> Session {
    let mut session = crate::twitch::tests::session();
    session.user_id = user;
    session
}

async fn manager() -> (tempfile::TempDir, Arc<AccountManager>) {
    let directory = tempfile::tempdir().unwrap();
    session(42).save(directory.path()).unwrap();
    let (manager, commands) = AccountManager::open(directory.path().into()).unwrap();
    mock_worker(&manager, manager.primary.clone(), commands);
    (directory, manager)
}

fn mock_worker(
    manager: &Arc<AccountManager>,
    app: Arc<Application>,
    mut commands: mpsc::Receiver<CommandRequest>,
) {
    let cancel = manager.shutdown.clone();
    manager.tasks.spawn(async move {
        loop {
            let request = tokio::select! { biased;
                _ = cancel.cancelled() => break,
                _ = app.shutdown.cancelled() => break,
                request = commands.recv() => match request { Some(request) => request, None => break },
            };
            if matches!(request.command, Command::Logout) {
                Session::remove(&app.data.path).unwrap();
                app.cancel_notifications();
                app.snapshot.write().await.login = Default::default();
            }
            let _ = request.complete.send(Ok(()));
        }
    });
}

async fn verified(manager: &AccountManager, user: u64, target: Option<&str>) -> String {
    let id = random_hex::<16>().unwrap();
    *manager.login.lock().await = Some(login::Attempt {
        view: LoginView {
            confirmed: false,
            id: id.clone(),
            target: target.map(str::to_owned),
            code: None,
            user_id: Some(user),
            name: Some(format!("user{user}")),
            avatar_url: None,
            error: None,
        },
        cancel: manager.shutdown.child_token(),
        confirmed: Arc::new(tokio::sync::Notify::new()),
        expires: Instant::now() + Duration::from_secs(600),
        session: Some(session(user)),
        profile: None,
    });
    id
}

async fn add(manager: &Arc<AccountManager>, user: u64) -> String {
    let attempt = verified(manager, user, None).await;
    let result = manager
        .request(Request::FinishLogin {
            attempt,
            label: format!("Account {user}"),
            copy_from: None,
            copy_auto_claim: false,
        })
        .await
        .unwrap();
    let id = result["id"].as_str().unwrap().to_owned();
    let mut entries = manager.entries.write().await;
    let entry = entries.get_mut(&id).unwrap();
    if let Some(commands) = entry.commands.take() {
        mock_worker(manager, entry.app.clone(), commands);
    }
    id
}

async fn target(manager: &AccountManager, id: &str) -> Target {
    Target {
        id: id.into(),
        revision: manager
            .account(id)
            .await
            .unwrap()
            .snapshot
            .read()
            .await
            .settings
            .revision
            .clone(),
    }
}

#[tokio::test]
async fn accepted_login_confirmation_stays_visible_and_only_wakes_oauth_once() {
    let (_directory, manager) = manager().await;
    let attempt = verified(&manager, 73, None).await;
    let notify = {
        let mut pending = manager.login.lock().await;
        let pending = pending.as_mut().unwrap();
        pending.view.user_id = None;
        pending.session = None;
        pending.view.code = Some(crate::dto::OAuthCode {
            code: "TESTCODE".into(),
            url: "https://www.twitch.tv/activate".into(),
        });
        pending.confirmed.clone()
    };
    assert!(!manager.overview().await.login.unwrap().confirmed);
    manager
        .request(Request::ConfirmLogin {
            attempt: attempt.clone(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), notify.notified())
        .await
        .unwrap();
    let state = manager.overview().await.login.unwrap();
    assert!(state.confirmed);
    assert!(state.user_id.is_none());
    assert!(state.code.is_some());
    manager
        .request(Request::ConfirmLogin {
            attempt: attempt.clone(),
        })
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), notify.notified())
            .await
            .is_err()
    );
    manager
        .request(Request::CancelLogin { attempt })
        .await
        .unwrap();
    assert!(manager.overview().await.login.is_none());
    manager.shutdown().await;
}

#[tokio::test]
async fn original_files_stay_in_place_and_profiles_have_independent_storage_and_identity() {
    let (directory, manager) = manager().await;
    assert_eq!(manager.primary.data.path, directory.path());
    let original = std::fs::read(directory.path().join("twitch_session.json")).unwrap();
    let id = add(&manager, 73).await;
    let second = manager.account(&id).await.unwrap();
    assert_eq!(
        second.data.path,
        directory.path().join("accounts").join(&id)
    );
    assert_eq!(
        std::fs::read(directory.path().join("twitch_session.json")).unwrap(),
        original
    );
    assert!(manager.identity_matches(PRIMARY, 42).await);
    assert!(!manager.identity_matches(PRIMARY, 73).await);
    assert!(manager.identity_matches(&id, 73).await);
    assert!(matches!(manager.sole_account().await, Err(Error::Required)));
    assert!(manager.account("../primary").await.is_err());
    assert!(Arc::ptr_eq(&manager.primary.telegram, &second.telegram));
    second
        .update_settings(json!({"games_to_watch":["Rust"],"mining_paused":true}))
        .await
        .unwrap();
    assert!(
        manager
            .primary
            .settings
            .read()
            .await
            .games_to_watch
            .is_empty()
    );
    assert!(!manager.primary.settings.read().await.mining_paused);
    manager.shutdown().await;
}

#[tokio::test]
async fn duplicate_login_does_not_replace_credentials_and_reauthentication_cannot_change_identity()
{
    let (directory, manager) = manager().await;
    let original = std::fs::read(directory.path().join("twitch_session.json")).unwrap();
    let duplicate = add(&manager, 42).await;
    assert_eq!(duplicate, PRIMARY);
    assert_eq!(manager.overview().await.accounts.len(), 1);
    assert_eq!(
        std::fs::read(directory.path().join("twitch_session.json")).unwrap(),
        original
    );
    let attempt = verified(&manager, 99, Some(PRIMARY)).await;
    assert!(matches!(
        manager
            .request(Request::FinishLogin {
                attempt,
                label: String::new(),
                copy_from: None,
                copy_auto_claim: false
            })
            .await,
        Err(AppError::Account(Error::Identity))
    ));
    assert_eq!(
        std::fs::read(directory.path().join("twitch_session.json")).unwrap(),
        original
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn bulk_actions_report_stale_accounts_without_replacing_newer_preferences() {
    let (_directory, manager) = manager().await;
    let id = add(&manager, 73).await;
    let targets = vec![target(&manager, PRIMARY).await, target(&manager, &id).await];
    let second = manager.account(&id).await.unwrap();
    second
        .update_settings(json!({"games_to_watch":["Rust"]}))
        .await
        .unwrap();
    let result = manager
        .request(Request::Pause {
            targets,
            paused: true,
        })
        .await
        .unwrap();
    assert_eq!(result["results"][0]["ok"], true);
    assert_eq!(result["results"][1]["error"], "settings_conflict");
    assert!(manager.primary.settings.read().await.mining_paused);
    assert!(!second.settings.read().await.mining_paused);
    assert_eq!(second.settings.read().await.games_to_watch, ["Rust"]);
    manager.shutdown().await;
}

#[tokio::test]
async fn copying_preferences_preserves_target_pause_display_connection_and_claim_setting() {
    let (_directory, manager) = manager().await;
    let id = add(&manager, 73).await;
    manager
        .primary
        .update_settings(
            json!({"games_to_watch":["Rust"],"auto_claim_drops":false,"inventory_list_view":true,
                "progress_time_format":"hours_minutes", "show_progress_time":false,
                "show_progress_countdown":true}),
        )
        .await
        .unwrap();
    let second = manager.account(&id).await.unwrap();
    second
        .update_settings(json!({"mining_paused":true}))
        .await
        .unwrap();
    let result = manager
        .request(Request::CopyPreferences {
            source: target(&manager, PRIMARY).await,
            targets: vec![target(&manager, &id).await],
            auto_claim: false,
        })
        .await
        .unwrap();
    assert_eq!(result["results"][0]["ok"], true);
    let settings = second.settings.read().await.clone();
    assert_eq!(settings.games_to_watch, ["Rust"]);
    assert!(settings.mining_paused && settings.auto_claim_drops);
    assert!(!settings.inventory_list_view);
    assert!(settings.show_progress_time);
    assert!(!settings.show_progress_countdown);
    assert_eq!(
        settings.progress_time_format,
        crate::config::ProgressTimeFormat::Minutes
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn connection_is_global_revision_checked_and_injected_on_restart() {
    let (directory, manager) = manager().await;
    let id = add(&manager, 73).await;
    let second = manager.account(&id).await.unwrap();
    assert!(
        second
            .update_settings(json!({"connection_quality":6}))
            .await
            .is_err()
    );
    let revision = manager.connection().await["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    let values = Connection {
        catalog_api_key: None,
        proxy: String::new(),
        connection_quality: 6,
        minimum_refresh_interval_minutes: 45,
    };
    manager
        .request(Request::Connection {
            revision: revision.clone(),
            values: values.clone(),
        })
        .await
        .unwrap();
    assert_eq!(manager.primary.settings.read().await.connection_quality, 6);
    assert_eq!(second.settings.read().await.connection_quality, 6);
    assert!(
        manager
            .request(Request::Connection { revision, values })
            .await
            .is_err()
    );
    drop(second);
    manager.shutdown().await;
    drop(manager);
    let (reopened, _) = AccountManager::open(directory.path().into()).unwrap();
    assert_eq!(
        reopened
            .account(&id)
            .await
            .unwrap()
            .settings
            .read()
            .await
            .minimum_refresh_interval_minutes,
        45
    );
    reopened.shutdown().await;
}

#[tokio::test]
async fn remove_preserves_data_and_readding_restores_the_same_identity() {
    let (_directory, manager) = manager().await;
    let id = add(&manager, 73).await;
    let second = manager.account(&id).await.unwrap();
    second
        .update_settings(json!({"games_to_watch":["Rust"]}))
        .await
        .unwrap();
    let path = second.data.path.clone();
    manager
        .request(Request::TelegramAccount { id: id.clone() })
        .await
        .unwrap();
    manager
        .request(Request::Remove { id: id.clone() })
        .await
        .unwrap();
    assert!(!path.join("twitch_session.json").exists());
    assert!(path.join("settings.json").exists());
    assert!(manager.account(&id).await.is_err());
    let restored = add(&manager, 73).await;
    assert_eq!(restored, id);
    assert_eq!(
        manager
            .account(&id)
            .await
            .unwrap()
            .settings
            .read()
            .await
            .games_to_watch,
        ["Rust"]
    );
    assert!(second.shutdown.is_cancelled());
    manager.shutdown().await;
}

#[tokio::test]
async fn old_unassigned_data_requires_explicit_association_and_is_not_used_for_a_new_account() {
    let directory = tempfile::tempdir().unwrap();
    atomic_json(
        &directory.path().join("completed_campaigns.json"),
        &json!({"version":1,"campaigns":[]}),
    )
    .unwrap();
    let (manager, commands) = AccountManager::open(directory.path().into()).unwrap();
    mock_worker(&manager, manager.primary.clone(), commands);
    assert!(manager.overview().await.accounts[0].unassigned);
    assert!(matches!(
        manager.start_login(Some(PRIMARY.into()), false).await,
        Err(Error::Identity)
    ));
    let id = add(&manager, 73).await;
    assert_ne!(id, PRIMARY);
    assert!(manager.overview().await.accounts[0].unassigned);
    manager.shutdown().await;
}

#[tokio::test]
async fn twenty_accounts_have_bounded_registry_and_shutdown_without_network() {
    let (_directory, manager) = manager().await;
    for user in 100..119 {
        add(&manager, user).await;
    }
    assert_eq!(manager.overview().await.accounts.len(), 20);
    let attempt = verified(&manager, 200, None).await;
    assert!(matches!(
        manager
            .request(Request::FinishLogin {
                attempt,
                label: String::new(),
                copy_from: None,
                copy_auto_claim: false
            })
            .await,
        Err(AppError::Account(Error::Limit))
    ));
    tokio::time::timeout(Duration::from_secs(3), manager.shutdown())
        .await
        .unwrap();
    assert!(manager.tasks.is_empty() && manager.writes.is_empty());
}

#[test]
fn corrupt_or_unsupported_registry_is_preserved() {
    let directory = tempfile::tempdir().unwrap();
    for original in [
        b"{broken".to_vec(),
        serde_json::to_vec(&json!({"version":20,"accounts":[]})).unwrap(),
    ] {
        std::fs::write(directory.path().join("accounts.json"), &original).unwrap();
        assert!(AccountManager::open(directory.path().into()).is_err());
        assert_eq!(
            std::fs::read(directory.path().join("accounts.json")).unwrap(),
            original
        );
    }
}

fn stored_profiles(directory: &std::path::Path) -> String {
    let id = "00000000000000000000000000000001".to_owned();
    let secondary = directory.join("accounts").join(&id);
    std::fs::create_dir_all(&secondary).unwrap();
    session(42).save(directory).unwrap();
    session(73).save(&secondary).unwrap();
    atomic_json(
        &directory.join("connection.json"),
        &Connection::from_settings(&Settings::default()),
    )
    .unwrap();
    atomic_json(
        &directory.join("accounts.json"),
        &json!({"version":1,"accounts":[
            {"id":"primary","user_id":42,"label":"","enabled":true,"removed":false},
            {"id":id,"user_id":73,"label":"","enabled":true,"removed":false}
        ]}),
    )
    .unwrap();
    id
}

#[tokio::test]
async fn removing_a_locked_unavailable_profile_cannot_delete_another_owners_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let id = stored_profiles(directory.path());
    let secondary = directory.path().join("accounts").join(&id);
    let lease = DataDirectory::open(&secondary).unwrap();
    let original = std::fs::read(secondary.join("twitch_session.json")).unwrap();
    let (manager, commands) = AccountManager::open(directory.path().into()).unwrap();
    mock_worker(&manager, manager.primary.clone(), commands);
    assert!(matches!(
        manager.request(Request::Remove { id: id.clone() }).await,
        Err(AppError::Account(Error::Storage))
    ));
    assert_eq!(
        std::fs::read(secondary.join("twitch_session.json")).unwrap(),
        original
    );
    assert!(!manager.registry.lock().await.find(&id).unwrap().removed);
    drop(lease);
    manager.shutdown().await;
}

#[tokio::test]
async fn repaired_removed_profile_can_be_readded_without_a_stale_failure_marker() {
    let directory = tempfile::tempdir().unwrap();
    let id = stored_profiles(directory.path());
    let settings = directory
        .path()
        .join("accounts")
        .join(&id)
        .join("settings.json");
    std::fs::write(&settings, "corrupt").unwrap();
    let (manager, commands) = AccountManager::open(directory.path().into()).unwrap();
    mock_worker(&manager, manager.primary.clone(), commands);
    assert!(manager.account(&id).await.is_err());
    manager
        .request(Request::Remove { id: id.clone() })
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "corrupt");
    atomic_json(&settings, &Settings::default()).unwrap();
    assert_eq!(add(&manager, 73).await, id);
    assert!(manager.account(&id).await.is_ok());
    assert!(!manager.failures.read().await.contains(&id));
    manager.shutdown().await;
}

#[tokio::test]
async fn corrupt_primary_settings_are_isolated_when_shared_connection_is_known() {
    let directory = tempfile::tempdir().unwrap();
    let id = stored_profiles(directory.path());
    std::fs::remove_file(directory.path().join("connection.json")).unwrap();
    let legacy = Settings {
        connection_quality: 5,
        ..Settings::default()
    };
    atomic_json(&directory.path().join("settings.json"), &legacy).unwrap();
    let (migrated, commands) = AccountManager::open(directory.path().into()).unwrap();
    drop(commands);
    assert!(directory.path().join("connection.json").exists());
    migrated.shutdown().await;
    drop(migrated);
    std::fs::write(directory.path().join("settings.json"), "corrupt").unwrap();
    let (manager, commands) = AccountManager::open(directory.path().into()).unwrap();
    drop(commands);
    assert!(manager.primary.shutdown.is_cancelled());
    assert!(matches!(
        manager.account(PRIMARY).await,
        Err(Error::Storage)
    ));
    let secondary = manager.account(&id).await.unwrap();
    assert_eq!(secondary.settings.read().await.connection_quality, 5);
    let commands = manager
        .entries
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .commands
        .take()
        .unwrap();
    mock_worker(&manager, secondary.clone(), commands);
    let result = manager
        .request(Request::Pause {
            targets: vec![target(&manager, &id).await],
            paused: true,
        })
        .await
        .unwrap();
    assert_eq!(result["results"][0]["ok"], true);
    assert!(secondary.settings.read().await.mining_paused);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("settings.json")).unwrap(),
        "corrupt"
    );
    // A failed local proxy connection is ordinary feedback, not account shutdown.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    assert_eq!(
        manager
            .primary
            .verify_proxy(&format!("http://{address}"))
            .await
            .unwrap()["success"],
        false
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn removing_an_account_invalidates_old_reauthentication_but_fresh_add_restores_it() {
    let (_directory, manager) = manager().await;
    let id = add(&manager, 73).await;
    let attempt = verified(&manager, 73, Some(&id)).await;
    let cancel = manager.login.lock().await.as_ref().unwrap().cancel.clone();
    manager
        .request(Request::Remove { id: id.clone() })
        .await
        .unwrap();
    assert!(cancel.is_cancelled());
    assert!(
        manager
            .request(Request::FinishLogin {
                attempt,
                label: String::new(),
                copy_from: None,
                copy_auto_claim: false,
            })
            .await
            .is_err()
    );
    assert!(manager.registry.lock().await.find(&id).unwrap().removed);
    assert_eq!(add(&manager, 73).await, id);
    manager.shutdown().await;
}

#[tokio::test]
async fn connection_changes_cancel_and_clear_pending_oauth_credentials() {
    let (_directory, manager) = manager().await;
    let attempt = verified(&manager, 73, None).await;
    let cancel = manager.login.lock().await.as_ref().unwrap().cancel.clone();
    let connection = manager.connection().await;
    let mut values: Connection = serde_json::from_value(connection["values"].clone()).unwrap();
    values.proxy = "http://127.0.0.1:9".into();
    manager
        .request(Request::Connection {
            revision: connection["revision"].as_str().unwrap().into(),
            values,
        })
        .await
        .unwrap();
    assert!(cancel.is_cancelled());
    {
        let pending = manager.login.lock().await;
        let login = pending.as_ref().unwrap();
        assert!(login.session.is_none());
        assert!(login.view.code.is_none());
        assert_eq!(
            login.view.error.as_deref(),
            Some("account_login_connection_changed")
        );
    }
    assert!(
        manager
            .request(Request::FinishLogin {
                attempt,
                label: String::new(),
                copy_from: None,
                copy_auto_claim: false,
            })
            .await
            .is_err()
    );
    assert_eq!(manager.registry.lock().await.accounts.len(), 1);
    manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_closes_manager_write_acceptance_before_returning() {
    let (directory, manager) = manager().await;
    let gate = manager.operations.acquire().await.unwrap();
    let owned = manager.clone();
    let write = tokio::spawn(async move {
        owned
            .request(Request::Rename {
                id: PRIMARY.into(),
                label: "late write".into(),
            })
            .await
    });
    tokio::task::yield_now().await;
    let owned = manager.clone();
    let shutdown = tokio::spawn(async move { owned.shutdown().await });
    manager.shutdown.cancelled().await;
    assert!(!shutdown.is_finished());
    drop(gate);
    assert!(write.await.unwrap().is_err());
    shutdown.await.unwrap();
    let registry = Registry::load(directory.path()).unwrap();
    assert_ne!(registry.find(PRIMARY).unwrap().label, "late write");
    assert!(manager.writes.is_empty());
    assert!(
        manager
            .request(Request::Rename {
                id: PRIMARY.into(),
                label: "after shutdown".into()
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn catalog_key_is_private_preserved_by_old_clients_and_explicitly_removable() {
    let (directory, manager) = manager().await;
    let mut values = Connection::from_settings(&*manager.primary.settings.read().await);
    values.catalog_api_key = Some("test-private-catalog-key".into());
    let revision = manager.connection().await["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    let result = manager
        .request(Request::Connection {
            revision,
            values: values.clone(),
        })
        .await
        .unwrap();
    assert_eq!(result["catalog_api_key_configured"], true);
    assert!(result["values"].get("catalog_api_key").is_none());
    assert!(!result.to_string().contains("test-private-catalog-key"));
    assert!(!format!("{values:?}").contains("test-private-catalog-key"));
    values.catalog_api_key = None;
    values.minimum_refresh_interval_minutes = 17;
    manager
        .request(Request::Connection {
            revision: result["revision"].as_str().unwrap().to_owned(),
            values,
        })
        .await
        .unwrap();
    manager.shutdown().await;
    drop(manager);
    let (manager, _) = AccountManager::open(directory.path().into()).unwrap();
    assert_eq!(
        manager.connection.read().await.0.catalog_api_key.as_deref(),
        Some("test-private-catalog-key")
    );
    assert_eq!(
        manager.connection().await["catalog_api_key_configured"],
        true
    );
    let mut values = manager.connection.read().await.0.clone();
    values.catalog_api_key = Some("bad\r\nheader".into());
    assert!(!values.valid());
    values.catalog_api_key = Some("".into());
    let revision = manager.connection().await["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    let result = manager
        .request(Request::Connection { revision, values })
        .await
        .unwrap();
    assert_eq!(result["catalog_api_key_configured"], false);
    assert_eq!(manager.connection.read().await.0.catalog_api_key, None);
    manager.shutdown().await;
}
