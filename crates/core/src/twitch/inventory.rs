use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::{TwitchClient, TwitchError, catalog::Catalog, diagnostics, operations::Operation};
use crate::{domain::Campaign, dto::InventoryStatus};

pub struct Inventory {
    pub campaigns: Vec<Campaign>,
    pub status: InventoryStatus,
    pub awards: HashMap<String, DateTime<Utc>>,
    pub rejected_account_ids: HashSet<String>,
}

impl TwitchClient {
    pub async fn inventory(&self) -> Result<Inventory, TwitchError> {
        let (response, public) =
            tokio::try_join!(self.gql(Operation::Inventory.request(json!({}))), async {
                match self.http.catalog().await {
                    Err(TwitchError::Cancelled) => Err(TwitchError::Cancelled),
                    result => Ok(result.ok()),
                }
            },)?;
        let inventory = response
            .pointer("/data/currentUser/inventory")
            .filter(|v| v.is_object())
            .ok_or_else(|| {
                diagnostics::invalid(
                    "Inventory",
                    "data.currentUser.inventory must be an object",
                    response.pointer("/data/currentUser/inventory"),
                )
            })?;
        let awards: HashMap<String, DateTime<Utc>> = values(&inventory["gameEventDrops"])
            .filter_map(|v| {
                Some((
                    v["id"].as_str()?.to_owned(),
                    v["lastAwardedAt"].as_str()?.parse().ok()?,
                ))
            })
            .collect();
        let now = Utc::now();
        let mut campaigns = BTreeMap::new();
        let mut account_ids = HashSet::new();
        let mut rejected_account_ids = HashSet::new();
        let mut account_complete = inventory["dropCampaignsInProgress"].is_array();
        let mut malformed_records = 0usize;
        let mut duplicates = 0usize;
        for record in values(&inventory["dropCampaignsInProgress"]) {
            // Even a damaged account record must not be replaced by public account assumptions.
            if let Some(id) = record["id"].as_str()
                && !account_ids.insert(id.to_owned())
            {
                duplicates += 1;
                account_complete = false;
                campaigns.remove(id);
                rejected_account_ids.insert(id.to_owned());
                continue;
            }
            match Campaign::parse(record, &awards, now) {
                Ok(campaign) => {
                    campaigns.insert(campaign.id.clone(), campaign);
                }
                Err(_) => {
                    malformed_records += 1;
                    account_complete = false;
                    if let Some(id) = record["id"].as_str() {
                        rejected_account_ids.insert(id.to_owned());
                    }
                }
            }
        }
        if !account_complete {
            tracing::warn!(
                operation = "Inventory",
                malformed_records,
                duplicates,
                collection_type = diagnostics::kind(inventory.get("dropCampaignsInProgress")),
                "Account inventory is partial"
            );
        }
        let catalog = public.and_then(|v| Catalog::parse(v, &awards, now).ok());
        let available = account_complete && catalog.as_ref().is_some_and(|c| c.complete);
        let catalog_updated_at = catalog.as_ref().map(|c| c.updated_at);
        if let Some(catalog) = catalog {
            for (id, campaign) in catalog.campaigns {
                if !account_ids.contains(&id) {
                    campaigns.insert(id, campaign);
                }
            }
        }
        Ok(Inventory {
            campaigns: campaigns.into_values().collect(),
            awards,
            rejected_account_ids,
            status: InventoryStatus {
                available,
                checked_at: Some(now),
                catalog_updated_at,
            },
        })
    }

    pub(crate) async fn batch(&self, queries: Vec<Value>) -> Result<Vec<Value>, TwitchError> {
        if queries.is_empty() {
            return Ok(vec![]);
        }
        let expected = queries.len();
        let response = self.gql(Value::Array(queries)).await?;
        match response {
            Value::Array(values) if values.len() == expected => Ok(values),
            Value::Object(_) if expected == 1 => Ok(vec![response]),
            _ => {
                tracing::warn!(
                    operation = "GraphQLBatch",
                    expected,
                    actual_type = diagnostics::kind(Some(&response)),
                    actual_count = response.as_array().map(Vec::len),
                    "Unexpected batch response shape"
                );
                Err(TwitchError::InvalidResponse)
            }
        }
    }
}

pub(crate) fn values(value: &Value) -> impl DoubleEndedIterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Settings,
        twitch::tests::{campaign_json, gql_mock, http, session},
    };
    use std::{sync::Arc, time::Duration};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    fn feed(records: Vec<Value>) -> Value {
        json!({"lastUpdatedAt":Utc::now().to_rfc3339(), "data":[{
            "gameId":"1", "gameBoxArtURL":"https://static-cdn.jtvnw.net/rust.png", "rewards":records
        }]})
    }

    async fn catalog(server: &MockServer, response: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path("/catalog"))
            .respond_with(response)
            .mount(server)
            .await;
    }

    async fn account(server: &MockServer, records: Value) {
        gql_mock(server, move |q| {
            assert_eq!(q["operationName"], "Inventory", "only account inventory goes to Twitch");
            json!({"data":{"currentUser":{"inventory":{"dropCampaignsInProgress":records,"gameEventDrops":[]}}}})
        }).await;
    }

    #[tokio::test]
    async fn duplicate_account_campaigns_are_excluded_in_every_order() {
        let unclaimed = campaign_json("ambiguous");
        let mut claimed = unclaimed.clone();
        claimed["timeBasedDrops"][0]["self"]["isClaimed"] = json!(true);
        let mut malformed = unclaimed.clone();
        malformed["timeBasedDrops"] = Value::Null;
        for records in [
            vec![unclaimed.clone(), claimed.clone()],
            vec![claimed.clone(), unclaimed.clone()],
            vec![claimed.clone(), unclaimed.clone(), claimed.clone()],
            vec![unclaimed.clone(), malformed.clone()],
            vec![malformed, unclaimed],
        ] {
            let server = MockServer::start().await;
            let mut records = records;
            records.push(campaign_json("healthy"));
            account(&server, json!(records)).await;
            catalog(
                &server,
                ResponseTemplate::new(200)
                    .set_body_json(feed(vec![claimed.clone(), campaign_json("public")])),
            )
            .await;
            let inventory = TwitchClient::new(Arc::new(http(&server)), &session())
                .inventory()
                .await
                .unwrap();
            assert!(!inventory.status.available);
            assert_eq!(
                inventory
                    .campaigns
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>(),
                ["healthy", "public"]
            );
        }
    }

    #[tokio::test]
    async fn public_catalog_and_account_inventory_are_isolated_and_discovery_never_selects_games() {
        let server = MockServer::start().await;
        account(&server, json!([campaign_json("owned")])).await;
        let mut records: Vec<_> = (0..145)
            .map(|i| campaign_json(&format!("public{i}")))
            .collect();
        let mut forged = campaign_json("owned");
        forged["self"]["isAccountConnected"] = false.into();
        forged["timeBasedDrops"][0]["self"]["isClaimed"] = true.into();
        records.push(forged);
        records[0]["timeBasedDrops"][0]["self"] =
            json!({"isClaimed":true,"currentMinutesWatched":60,"dropInstanceID":"forged"});
        catalog(
            &server,
            ResponseTemplate::new(200).set_body_json(feed(records)),
        )
        .await;
        let http = Arc::new(http(&server));
        http.jar
            .add_cookie_str("auth-token=private; Path=/", &http.endpoints.tv);
        let client = TwitchClient::new(http, &session());
        let inventory = client.inventory().await.unwrap();
        assert!(inventory.status.available);
        assert!(inventory.status.catalog_updated_at.is_some());
        assert_eq!(inventory.campaigns.len(), 146);
        let owned = inventory
            .campaigns
            .iter()
            .find(|c| c.id == "owned")
            .unwrap();
        assert_eq!(owned.linked, Some(true));
        assert_eq!(owned.drops[0].confirmed_minutes, 12);
        assert!(!owned.drops[0].claimed);
        let public = inventory
            .campaigns
            .iter()
            .find(|c| c.id == "public0")
            .unwrap();
        assert_eq!(public.linked, None);
        assert_eq!(
            public.game.image_url,
            "https://static-cdn.jtvnw.net/rust.png"
        );
        assert!(!public.drops[0].claimed);
        assert!(public.drops[0].claim_id.is_none() && public.drops[0].confirmed_at.is_none());
        assert_eq!(public.drops[0].confirmed_minutes, 0);
        assert!(
            crate::domain::wanted_items(&inventory.campaigns, &Settings::default(), Utc::now())
                .is_empty()
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            2,
            "one catalog GET and one Inventory query, no scans/details"
        );
        let request = requests
            .iter()
            .find(|r| r.url.path() == "/catalog")
            .unwrap();
        for header in [
            "authorization",
            "cookie",
            "client-id",
            "client-session-id",
            "x-device-id",
            "origin",
        ] {
            assert!(
                !request.headers.contains_key(header),
                "public request leaked {header}"
            );
        }
        assert!(request.body.is_empty() && request.url.query().is_none());
    }

    #[tokio::test]
    async fn unavailable_stale_invalid_and_empty_public_feeds_keep_account_evidence() {
        for mode in [
            "empty",
            "unauthorized",
            "redirect",
            "malformed",
            "stale",
            "future",
            "null_groups",
            "invalid_neighbor",
        ] {
            let server = MockServer::start().await;
            account(&server, json!([campaign_json("owned")])).await;
            let mut payload = feed(vec![]);
            match mode {
                "stale" => {
                    payload["lastUpdatedAt"] = (Utc::now() - chrono::Duration::minutes(31))
                        .to_rfc3339()
                        .into()
                }
                "future" => {
                    payload["lastUpdatedAt"] = (Utc::now() + chrono::Duration::minutes(6))
                        .to_rfc3339()
                        .into()
                }
                "null_groups" => payload["data"] = Value::Null,
                "invalid_neighbor" => {
                    payload["data"][0]["rewards"] = json!([null, campaign_json("new")])
                }
                _ => {}
            }
            let response = match mode {
                "unauthorized" => ResponseTemplate::new(401),
                "redirect" => ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/private", server.uri())),
                "malformed" => ResponseTemplate::new(200).set_body_string("not json"),
                _ => ResponseTemplate::new(200).set_body_json(payload),
            };
            catalog(&server, response).await;
            let inventory = TwitchClient::new(Arc::new(http(&server)), &session())
                .inventory()
                .await
                .unwrap();
            assert_eq!(inventory.status.available, mode == "empty", "{mode}");
            assert_eq!(
                inventory.campaigns.len(),
                if mode == "invalid_neighbor" { 2 } else { 1 }
            );
            let owned = inventory
                .campaigns
                .iter()
                .find(|c| c.id == "owned")
                .unwrap();
            assert_eq!(owned.drops[0].confirmed_minutes, 12);
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.url.path() != "/private")
            );
        }
    }

    #[tokio::test]
    async fn malformed_account_records_are_partial_and_cannot_be_replaced_by_public_assumptions() {
        for mode in [
            "damaged",
            "null_collection",
            "null_neighbor",
            "missing_id",
            "missing_acl",
            "wrong_acl",
            "missing_dependencies",
            "wrong_dependencies",
            "missing_benefits",
        ] {
            let server = MockServer::start().await;
            let mut damaged = campaign_json("damaged");
            damaged["timeBasedDrops"] = Value::Null;
            if !matches!(
                mode,
                "damaged" | "null_collection" | "null_neighbor" | "missing_id"
            ) {
                damaged = campaign_json("damaged");
                match mode {
                    "missing_acl" => {
                        damaged["allow"].as_object_mut().unwrap().remove("channels");
                    }
                    "wrong_acl" => damaged["allow"]["isEnabled"] = json!("true"),
                    "missing_dependencies" => {
                        damaged["timeBasedDrops"][0]
                            .as_object_mut()
                            .unwrap()
                            .remove("preconditionDrops");
                    }
                    "wrong_dependencies" => {
                        damaged["timeBasedDrops"][0]["preconditionDrops"] = json!({})
                    }
                    _ => damaged["timeBasedDrops"][0]["benefitEdges"] = Value::Null,
                }
            }
            let records = match mode {
                "null_collection" => Value::Null,
                "null_neighbor" => json!([null, campaign_json("owned")]),
                "missing_id" => json!([{"id":null}, campaign_json("owned")]),
                _ => json!([damaged, campaign_json("owned")]),
            };
            account(&server, records).await;
            let mut public_damaged = campaign_json("damaged");
            public_damaged["allow"] = json!({"isEnabled":false});
            catalog(
                &server,
                ResponseTemplate::new(200)
                    .set_body_json(feed(vec![public_damaged, campaign_json("new")])),
            )
            .await;
            let inventory = TwitchClient::new(Arc::new(http(&server)), &session())
                .inventory()
                .await
                .unwrap();
            assert!(!inventory.status.available);
            assert!(inventory.campaigns.iter().any(|c| c.id == "new"));
            if !matches!(mode, "null_collection" | "null_neighbor" | "missing_id") {
                assert!(!inventory.campaigns.iter().any(|c| c.id == "damaged"));
            }
            if mode != "null_collection" {
                assert_eq!(
                    inventory
                        .campaigns
                        .iter()
                        .find(|c| c.id == "owned")
                        .unwrap()
                        .drops[0]
                        .confirmed_minutes,
                    12
                );
            }
        }
    }

    #[test]
    fn public_catalog_accepts_omitted_channels_only_when_explicitly_disabled() {
        let now = Utc::now();
        let mut raw = campaign_json("public");
        raw["allow"] = json!({"isEnabled":false});
        raw["timeBasedDrops"][0]["preconditionDrops"] = Value::Null;
        // This compact public-feed shape must not relax account inventory parsing.
        assert!(Campaign::parse(&raw, &HashMap::new(), now).is_err());
        let catalog = Catalog::parse(feed(vec![raw.clone()]), &HashMap::new(), now).unwrap();
        assert!(catalog.complete);
        let campaign = &catalog.campaigns["public"];
        assert!(campaign.allowed_channels.is_empty());
        assert!(campaign.can_mine(
            &Settings {
                games_to_watch: vec!["Rust".into()],
                ..Settings::default()
            },
            now
        ));
        assert_eq!(campaign.linked, None);
        assert_eq!(campaign.drops[0].confirmed_minutes, 0);
        assert!(!campaign.drops[0].claimed);
        assert!(campaign.drops[0].claim_id.is_none());
        for acl in [
            Value::Null,
            json!({}),
            json!({"channels":[]}),
            json!({"isEnabled":null}),
            json!({"isEnabled":"false"}),
            json!({"isEnabled":false,"channels":{}}),
            json!({"isEnabled":true}),
            json!({"isEnabled":true,"channels":null}),
            json!({"isEnabled":true,"channels":[]}),
            json!({"isEnabled":true,"channels":[null,{"id":"10","name":"streamer"}]}),
        ] {
            raw["allow"] = acl.clone();
            let catalog = Catalog::parse(feed(vec![raw.clone()]), &HashMap::new(), now).unwrap();
            assert!(!catalog.complete, "{acl}");
            assert!(catalog.campaigns.is_empty(), "{acl}");
        }
    }

    #[test]
    fn public_metadata_keeps_real_restrictions_and_only_twitch_awards_can_infer_claims() {
        let now = Utc::now();
        let mut raw = campaign_json("new");
        raw["allow"] = json!({"isEnabled":true,"channels":[{"id":"10","name":"only_streamer"}]});
        raw["timeBasedDrops"][0]["preconditionDrops"] = json!([{"id":"prerequisite"}]);
        let awards = HashMap::from([("benefit-new".to_owned(), now)]);
        let catalog = Catalog::parse(feed(vec![raw]), &awards, now).unwrap();
        let campaign = &catalog.campaigns["new"];
        assert!(campaign.drops[0].claimed);
        assert!(campaign.drops[0].claim_id.is_none());
        assert_eq!(campaign.linked, None);
        assert_eq!(campaign.allowed_channels[0].login, "only_streamer");
        assert_eq!(campaign.drops[0].prerequisites, ["prerequisite"]);
        for mode in [
            "missing_acl",
            "empty_acl",
            "null_acl_neighbor",
            "missing_dependencies",
            "missing_benefits",
            "duplicate",
            "invalid_dates",
            "invalid_game",
            "channel_url",
        ] {
            let mut broken = campaign_json("bad");
            match mode {
                "missing_acl" => broken["allow"] = Value::Null,
                "empty_acl" => broken["allow"] = json!({"isEnabled":true,"channels":[]}),
                "null_acl_neighbor" => {
                    broken["allow"] =
                        json!({"isEnabled":true,"channels":[null,{"id":"10","name":"known"}]})
                }
                "missing_dependencies" => {
                    broken["timeBasedDrops"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("preconditionDrops");
                }
                "missing_benefits" => broken["timeBasedDrops"][0]["benefitEdges"] = Value::Null,
                "invalid_dates" => broken["endAt"] = broken["startAt"].clone(),
                "invalid_game" => broken["game"] = json!("not an object"),
                "channel_url" => {
                    broken["allow"] = json!({"isEnabled":true,"channels":[{"id":"10","name":"https://outside.invalid/path"}]})
                }
                _ => {}
            }
            let mut records = vec![campaign_json("healthy"), broken.clone()];
            if mode == "duplicate" {
                records.push(broken);
            }
            let catalog = Catalog::parse(feed(records), &HashMap::new(), now).unwrap();
            assert!(!catalog.complete, "{mode}");
            assert_eq!(catalog.campaigns.len(), 1, "{mode}");
            assert!(catalog.campaigns.contains_key("healthy"));
        }
    }

    #[tokio::test]
    async fn twitch_auth_and_cancellation_interrupt_slow_public_requests() {
        let server = MockServer::start().await;
        gql_mock(&server, |_| json!({"errors":[{"message":"Unauthorized"}]})).await;
        catalog(
            &server,
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(20))
                .set_body_json(feed(vec![])),
        )
        .await;
        let http = Arc::new(http(&server));
        let client = TwitchClient::new(http.clone(), &session());
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), client.inventory())
                .await
                .unwrap(),
            Err(TwitchError::Unauthorized)
        ));
        http.cancel.cancel();
        assert!(matches!(
            client.inventory().await,
            Err(TwitchError::Cancelled)
        ));
    }

    #[tokio::test]
    async fn public_body_and_record_limits_reject_excess_without_losing_inventory() {
        let server = MockServer::start().await;
        account(&server, json!([campaign_json("owned")])).await;
        catalog(
            &server,
            ResponseTemplate::new(200).set_body_bytes(vec![b' '; super::super::MAX_BODY + 1]),
        )
        .await;
        let inventory = TwitchClient::new(Arc::new(http(&server)), &session())
            .inventory()
            .await
            .unwrap();
        assert!(!inventory.status.available);
        assert_eq!(inventory.campaigns.len(), 1);
        assert!(
            Catalog::parse(feed(vec![Value::Null; 2001]), &HashMap::new(), Utc::now()).is_err()
        );
    }
}
