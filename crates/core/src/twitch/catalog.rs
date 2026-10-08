use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{RetryPolicy, TwitchError, TwitchHttp, diagnostics};
use crate::domain::Campaign;

const MAX_CAMPAIGNS: usize = 2000;

pub(super) struct Catalog {
    pub campaigns: BTreeMap<String, Campaign>,
    pub updated_at: DateTime<Utc>,
    pub complete: bool,
}

impl TwitchHttp {
    #[tracing::instrument(skip_all, fields(operation = "PublicCatalog"))]
    pub(super) async fn catalog(&self) -> Result<Value, TwitchError> {
        // One inventory job owns this request. Bound retries and body reads together.
        tokio::time::timeout(Duration::from_secs(30), async {
            let response = self
                .execute_with(
                    &self.catalog_client,
                    self.catalog_client
                        .get(self.endpoints.catalog.clone())
                        .header("Accept", "application/json"),
                    RetryPolicy::Replay,
                )
                .await?;
            // A public-feed 401/403 is not a Twitch logout.
            if !response.status().is_success() {
                return Err(TwitchError::InvalidResponse);
            }
            diagnostics::json(response.body(), response.status().as_u16())
        })
        .await
        .map_err(|_| {
            tracing::warn!(
                timeout_seconds = 30,
                "Public catalog request exceeded total deadline"
            );
            TwitchError::Network
        })?
    }
}

impl Catalog {
    pub fn parse(
        payload: Value,
        awards: &HashMap<String, DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<Self, TwitchError> {
        let updated_at: DateTime<Utc> = payload["lastUpdatedAt"]
            .as_str()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| {
                diagnostics::invalid(
                    "PublicCatalog",
                    "lastUpdatedAt must be an RFC3339 timestamp",
                    payload.get("lastUpdatedAt"),
                )
            })?;
        if updated_at < now - chrono::Duration::minutes(30)
            || updated_at > now + chrono::Duration::minutes(5)
        {
            tracing::warn!(
                operation = "PublicCatalog",
                age_seconds = (now - updated_at).num_seconds(),
                "Catalog timestamp is outside the freshness window (-300..1800 seconds)"
            );
            return Err(TwitchError::InvalidResponse);
        }
        let groups = payload["data"].as_array().ok_or_else(|| {
            diagnostics::invalid(
                "PublicCatalog",
                "data must be an array",
                payload.get("data"),
            )
        })?;
        if groups.len() > MAX_CAMPAIGNS {
            tracing::warn!(
                operation = "PublicCatalog",
                groups = groups.len(),
                limit = MAX_CAMPAIGNS,
                "Catalog group limit exceeded"
            );
            return Err(TwitchError::InvalidResponse);
        }
        let mut catalog = Self {
            campaigns: BTreeMap::new(),
            updated_at,
            complete: true,
        };
        let mut count = 0;
        let mut seen = HashSet::new();
        let mut invalid_groups = 0usize;
        let mut invalid_records = 0usize;
        let mut duplicates = 0usize;
        for group in groups {
            let Some(records) = group["rewards"].as_array() else {
                invalid_groups += 1;
                catalog.complete = false;
                continue;
            };
            count += records.len();
            if count > MAX_CAMPAIGNS {
                tracing::warn!(
                    operation = "PublicCatalog",
                    records = count,
                    limit = MAX_CAMPAIGNS,
                    "Catalog campaign limit exceeded"
                );
                return Err(TwitchError::InvalidResponse);
            }
            for record in records {
                let Some(campaign) = public_campaign(record.clone(), group, awards, now) else {
                    invalid_records += 1;
                    catalog.complete = false;
                    continue;
                };
                if !seen.insert(campaign.id.clone()) {
                    duplicates += 1;
                    catalog.complete = false;
                    catalog.campaigns.remove(&campaign.id);
                    continue;
                }
                if campaign.active(now) || campaign.upcoming(now) {
                    catalog.campaigns.insert(campaign.id.clone(), campaign);
                }
            }
        }
        if !catalog.complete {
            tracing::warn!(
                operation = "PublicCatalog",
                invalid_groups,
                invalid_records,
                duplicates,
                "Public catalog is partial: malformed groups/records or duplicate IDs"
            );
        }
        Ok(catalog)
    }
}

fn public_campaign(
    mut record: Value,
    group: &Value,
    awards: &HashMap<String, DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<Campaign> {
    record.as_object_mut()?.remove("self");
    record["game"].as_object()?;
    if !matches!(
        record["status"].as_str()?,
        "ACTIVE" | "UPCOMING" | "EXPIRED"
    ) {
        return None;
    }
    let restricted = record["allow"]["isEnabled"].as_bool()?;
    // The public feed omits the unused channel list when restrictions are disabled.
    // Normalize only that explicit case; account inventory still requires the field.
    if !restricted && record["allow"].get("channels").is_none() {
        record["allow"]["channels"] = Value::Array(vec![]);
    }
    if restricted
        && record["allow"]["channels"]
            .as_array()
            .is_none_or(|channels| channels.is_empty() || channels.iter().any(Value::is_null))
    {
        return None;
    }
    for drop in record["timeBasedDrops"].as_array_mut()? {
        drop.as_object_mut()?.remove("self");
        // Missing dependency or benefit data cannot be interpreted as unrestricted.
        if !(drop.get("preconditionDrops")?.is_null() || drop["preconditionDrops"].is_array())
            || drop["benefitEdges"].as_array()?.is_empty()
        {
            return None;
        }
    }
    if record["game"]["boxArtURL"].as_str().is_none() && record["game"]["id"] == group["gameId"] {
        record["game"]["boxArtURL"] = group["gameBoxArtURL"].clone();
    }
    let campaign = Campaign::parse(&record, awards, now).ok()?;
    if campaign.game.id == 0
        || campaign.allowed_channels.iter().any(|channel| {
            channel.id == 0
                || super::channels::channel_login(&channel.login)
                    .is_none_or(|login| !login.eq_ignore_ascii_case(&channel.login))
        })
        || campaign.starts_at >= campaign.ends_at
        || campaign
            .drops
            .iter()
            .any(|drop| drop.starts_at >= drop.ends_at)
        || restricted && campaign.allowed_channels.is_empty()
    {
        return None;
    }
    Some(campaign)
}
