use std::{collections::HashSet, path::Path};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    config::Settings,
    store::{atomic_json, read_json},
};

pub const PRIMARY: &str = "primary";
pub const MAX_ACCOUNTS: usize = 20;
const REGISTRY: &str = "accounts.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Account {
    pub id: String,
    pub user_id: Option<u64>,
    pub label: String,
    pub enabled: bool,
    pub removed: bool,
    #[serde(default)]
    pub unassigned: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Registry {
    version: u32,
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub telegram_account: Option<String>,
}

pub fn valid_id(id: &str) -> bool {
    id == PRIMARY
        || id.len() == 32
            && id
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub(super) fn valid_label(label: &str) -> bool {
    label.len() <= 100 && !label.chars().any(char::is_control)
}

impl Registry {
    pub fn load(root: &Path) -> Result<Self> {
        if let Some(saved) = read_json::<Self>(&root.join(REGISTRY))? {
            saved.validate()?;
            return Ok(saved);
        }
        // Read only the identity. Session validation/repair remains owned by the miner.
        let identity = read_json::<serde_json::Value>(&root.join("twitch_session.json"))
            .ok()
            .flatten()
            .and_then(|value| value["user_id"].as_u64())
            .filter(|id| *id > 0);
        let previous_data = [
            "drop_history.json",
            "completed_campaigns.json",
            "pending_claims.json",
        ]
        .iter()
        .any(|name| root.join(name).exists());
        let unassigned = identity.is_none() && previous_data;
        let result = Self {
            version: 1,
            accounts: vec![Account {
                id: PRIMARY.into(),
                user_id: identity,
                label: String::new(),
                enabled: !unassigned,
                removed: false,
                unassigned,
            }],
            telegram_account: Some(PRIMARY.into()),
        };
        result.save(root)?;
        Ok(result)
    }

    fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        let mut users = HashSet::new();
        ensure!(
            self.version == 1 && self.accounts.len() <= 100,
            "invalid account registry; original file preserved"
        );
        ensure!(
            self.accounts.iter().filter(|a| a.id == PRIMARY).count() == 1,
            "missing original account; registry preserved"
        );
        ensure!(
            self.accounts.iter().filter(|a| !a.removed).count() <= MAX_ACCOUNTS,
            "too many active accounts"
        );
        for account in &self.accounts {
            ensure!(
                valid_id(&account.id)
                    && ids.insert(&account.id)
                    && valid_label(&account.label)
                    && account.user_id.is_none_or(|id| id > 0 && users.insert(id))
                    && (!account.unassigned || account.id == PRIMARY && account.user_id.is_none())
                    && (!account.removed || !account.enabled),
                "invalid account registry; original file preserved"
            );
        }
        ensure!(
            self.telegram_account
                .as_ref()
                .is_none_or(|id| self.accounts.iter().any(|a| &a.id == id && !a.removed)),
            "invalid Telegram account selection"
        );
        Ok(())
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        self.validate()?;
        atomic_json(&root.join(REGISTRY), self).context("cannot save accounts")
    }

    pub fn find(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }
    pub fn find_mut(&mut self, id: &str) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.id == id)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    #[serde(default)]
    pub catalog_api_key: Option<String>,
    pub proxy: String,
    pub connection_quality: u8,
    pub minimum_refresh_interval_minutes: u32,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field(
                "catalog_api_key_configured",
                &self.catalog_api_key.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl Connection {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            catalog_api_key: None,
            proxy: settings.proxy.clone(),
            connection_quality: settings.connection_quality,
            minimum_refresh_interval_minutes: settings.minimum_refresh_interval_minutes,
        }
    }
    pub fn apply(&self, settings: &mut Settings) {
        settings.proxy.clone_from(&self.proxy);
        settings.connection_quality = self.connection_quality;
        settings.minimum_refresh_interval_minutes = self.minimum_refresh_interval_minutes;
    }
    pub fn valid(&self) -> bool {
        crate::config::validate_proxy(&self.proxy).is_ok()
            && self.catalog_api_key.as_ref().is_none_or(|key| {
                key.len() <= 512 && key.bytes().all(|byte| (33..=126).contains(&byte))
            })
            && self.proxy.len() <= 4096
            && (1..=6).contains(&self.connection_quality)
            && (1..=1440).contains(&self.minimum_refresh_interval_minutes)
    }
    pub fn load(root: &Path, legacy: &Settings) -> Result<Self> {
        let saved = read_json::<Self>(&root.join("connection.json"))?;
        let value = saved.clone().unwrap_or_else(|| Self::from_settings(legacy));
        ensure!(
            value.valid(),
            "invalid connection settings; original file preserved"
        );
        // Isolate later primary-profile damage without requiring a manual Connection save.
        if saved.is_none() {
            value.save(root)?;
        }
        Ok(value)
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        ensure!(self.valid(), "invalid connection settings");
        atomic_json(&root.join("connection.json"), self)
    }
}
