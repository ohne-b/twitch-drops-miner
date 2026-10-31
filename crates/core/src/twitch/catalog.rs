use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{TwitchError, TwitchHttp, diagnostics};
use crate::domain::Campaign;

const MAX_CAMPAIGNS: usize = 2000;

pub(super) struct Catalog {
    pub campaigns: BTreeMap<String, Campaign>,
    pub updated_at: DateTime<Utc>,
    pub complete: bool,
}

pub(super) struct CatalogFetch {
    pub result: Result<Value, TwitchError>,
    pub cooldown: Duration,
}

pub(super) fn interval(key: Option<&str>) -> Duration {
    Duration::from_secs(if key.is_some() { 60 } else { 300 })
}

pub(super) fn fresh(payload: &Value) -> bool {
    payload["lastUpdatedAt"]
        .as_str()
        .and_then(|value| value.parse::<DateTime<Utc>>().ok())
        .is_some_and(|updated| {
            let now = Utc::now();
            updated >= now - chrono::Duration::minutes(30)
                && updated <= now + chrono::Duration::minutes(5)
        })
}

fn cooldown(headers: &http::HeaderMap, minimum: Duration) -> Duration {
    ["ratelimit-reset", "retry-after"]
        .into_iter()
        .fold(minimum, |delay, name| {
            // Both documented headers are delta seconds, not Unix timestamps. Bound
            // hostile values before adding to Instant; never retry a rejected request.
            let seconds = headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(0);
            delay.max(Duration::from_secs(seconds.min(7 * 24 * 60 * 60)))
        })
}

impl TwitchHttp {
    pub(super) async fn catalog(&self) -> Result<std::sync::Arc<Value>, TwitchError> {
        if let Some(shared) = &self.shared {
            shared.catalog(self).await
        } else {
            self.fetch_catalog(None)
                .await
                .result
                .map(std::sync::Arc::new)
        }
    }

    #[tracing::instrument(skip_all, fields(operation = "PublicCatalog"))]
    pub(super) async fn fetch_catalog(&self, key: Option<&str>) -> CatalogFetch {
        let mut delay = interval(key);
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            let mut request = self
                .catalog_client
                .get(self.endpoints.catalog.clone())
                .header("Accept", "application/json");
            if let Some(key) = key {
                let mut value =
                    http::HeaderValue::from_str(key).map_err(|_| TwitchError::Configuration)?;
                value.set_sensitive(true);
                request = request.header("X-API-Key", value);
            }
            let request = request.build().map_err(|_| TwitchError::Configuration)?;
            let capture = self.diagnostics.request(&request, None);
            let started = tokio::time::Instant::now();
            let response = tokio::select! { biased;
                _ = self.cancel.cancelled() => return Err(TwitchError::Cancelled),
                response = self.catalog_client.execute(request) => response.map_err(|error| {
                    capture.network(&error, "send", 1, started.elapsed().as_millis());
                    TwitchError::Network
                })?,
            };
            // Record the server cooldown even if the body fails or is cancelled.
            delay = cooldown(response.headers(), delay);
            let status = response.status();
            if !status.is_success() {
                tracing::warn!(
                    status = status.as_u16(),
                    retry_after_seconds = delay.as_secs(),
                    "Public catalog request failed; waiting before the next request"
                );
                return Err(TwitchError::Status(status.as_u16()));
            }
            let response = self.read_response(response, 1, &capture, started).await?;
            let payload = diagnostics::json(response.body(), status.as_u16())?;
            // Validate before replacing a usable cache. Account evidence is still
            // applied separately by each inventory reader.
            Catalog::parse(&payload, &HashMap::new(), Utc::now())?;
            Ok(payload)
        })
        .await
        .unwrap_or_else(|_| {
            tracing::warn!(
                timeout_seconds = 30,
                "Public catalog request exceeded total deadline"
            );
            Err(TwitchError::Network)
        });
        CatalogFetch {
            result,
            cooldown: delay,
        }
    }
}

impl Catalog {
    pub fn parse(
        payload: impl std::borrow::Borrow<Value>,
        awards: &HashMap<String, DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<Self, TwitchError> {
        let payload = payload.borrow();
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

#[cfg(test)]
mod rate_tests {
    use super::*;

    #[test]
    fn cooldown_headers_are_delta_seconds_and_bounded() {
        let mut headers = http::HeaderMap::new();
        for value in ["invalid", "-1", "18446744073709551616", "0", "1"] {
            headers.insert("ratelimit-reset", value.parse().unwrap());
            assert_eq!(cooldown(&headers, interval(None)), Duration::from_secs(300));
        }
        headers.insert("ratelimit-reset", "3600".parse().unwrap());
        headers.insert("retry-after", "7200".parse().unwrap());
        assert_eq!(
            cooldown(&headers, interval(None)),
            Duration::from_secs(7200)
        );
        headers.insert("retry-after", u64::MAX.to_string().parse().unwrap());
        assert_eq!(
            cooldown(&headers, interval(None)),
            Duration::from_secs(604800)
        );
    }
}
