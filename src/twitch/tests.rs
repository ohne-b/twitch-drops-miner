use std::{sync::Arc, time::Duration};

use http::Method;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

use super::{
    CLIENT_ID, CLIENT_ORIGIN, Endpoints, RetryPolicy, TwitchClient, TwitchError, TwitchHttp,
    gql_errors, oauth::Session,
};

mod protocol_regressions {
    use super::super::operations::Operation;
    use super::*;

    #[tokio::test]
    async fn persisted_operation_variables_match_legacy_contract() {
        let server = MockServer::start().await;
        gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
            "Inventory" => json!({"data":{"currentUser":{"inventory":{"dropCampaignsInProgress":[],"gameEventDrops":[]}}}}),
            "DropCurrentSessionContext" => json!({"data":{"currentUser":{"dropCurrentSession":null}}}),
            _ => unreachable!(),
        }).await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        client.inventory().await.unwrap();
        client.current_drop(10).await.unwrap();
        let bodies: Vec<serde_json::Value> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .map(|r| r.body_json().unwrap())
            .collect();
        assert_eq!(
            bodies[0]["variables"]["fetchRewardCampaigns"], false,
            "Inventory request: {}",
            bodies[0]
        );
        assert_eq!(bodies[1]["variables"]["channelLogin"], "");
        assert_eq!(bodies.len(), 2);
    }

    #[tokio::test]
    async fn current_drop_accepts_null_and_empty_sessions() {
        for drop in [
            json!(null),
            json!({
                "__typename":"DropCurrentSession", "channel":null,
                "currentMinutesWatched":0, "dropID":"", "game":null,
                "requiredMinutesWatched":0
            }),
        ] {
            let server = MockServer::start().await;
            gql_mock(
                &server,
                move |_| json!({"data":{"currentUser":{"dropCurrentSession":drop}}}),
            )
            .await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            assert_eq!(client.current_drop(10).await, Ok(None));
        }
    }

    #[tokio::test]
    async fn current_drop_accepts_progress_from_the_requested_channel() {
        let server = MockServer::start().await;
        gql_mock(&server, |_| {
            json!({"data":{"currentUser":{"dropCurrentSession":{
                "channel":{"id":"10"}, "dropID":"reward", "currentMinutesWatched":3
            }}}})
        })
        .await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        assert_eq!(
            client.current_drop(10).await,
            Ok(Some(("reward".into(), 3)))
        );
    }

    #[tokio::test]
    async fn current_drop_rejects_malformed_empty_sessions() {
        let empty = json!({
            "channel":null, "dropID":"", "currentMinutesWatched":0,
            "game":null, "requiredMinutesWatched":0
        });
        for (field, value) in [
            ("channel", json!({})),
            ("channel", json!({"id":"10"})),
            ("dropID", json!("reward")),
            ("dropID", json!(null)),
            ("currentMinutesWatched", json!(1)),
            ("currentMinutesWatched", json!("0")),
            ("game", json!({"id":"1"})),
            ("requiredMinutesWatched", json!(60)),
            ("requiredMinutesWatched", json!(null)),
        ] {
            let mut drop = empty.clone();
            drop[field] = value;
            let server = MockServer::start().await;
            gql_mock(
                &server,
                move |_| json!({"data":{"currentUser":{"dropCurrentSession":drop}}}),
            )
            .await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            assert_eq!(
                client.current_drop(10).await,
                Err(TwitchError::InvalidResponse)
            );
        }
        for field in [
            "channel",
            "dropID",
            "currentMinutesWatched",
            "game",
            "requiredMinutesWatched",
        ] {
            let mut drop = empty.clone();
            drop.as_object_mut().unwrap().remove(field);
            let server = MockServer::start().await;
            gql_mock(
                &server,
                move |_| json!({"data":{"currentUser":{"dropCurrentSession":drop}}}),
            )
            .await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            assert_eq!(
                client.current_drop(10).await,
                Err(TwitchError::InvalidResponse)
            );
        }
    }

    #[tokio::test]
    async fn current_drop_rejects_progress_from_another_channel() {
        let server = MockServer::start().await;
        gql_mock(&server, |_| {
            json!({"data":{"currentUser":{"dropCurrentSession":{
                "channel":{"id":"11"}, "dropID":"old-reward", "currentMinutesWatched":60
            }}}})
        })
        .await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        assert_eq!(client.current_drop(10).await.unwrap(), None);
    }

    #[tokio::test]
    async fn current_drop_requires_a_valid_reported_channel() {
        for channel in [json!(null), json!({}), json!({"id":"not-an-id"})] {
            let server = MockServer::start().await;
            gql_mock(&server, move |_| {
                json!({"data":{"currentUser":{"dropCurrentSession":{
                    "channel":channel, "dropID":"reward", "currentMinutesWatched":3
                }}}})
            })
            .await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            assert_eq!(
                client.current_drop(10).await,
                Err(TwitchError::InvalidResponse)
            );
        }
    }

    #[tokio::test]
    async fn null_ancestor_keeps_independent_batch_neighbor() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/gql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
              {"data":{"user":null},"errors":[{"message":"server error","path":["user","stream"]}]},
              {"data":{"user":{"stream":{"id":"valid-neighbor"}}}}
            ])))
            .mount(&server)
            .await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let response = client
            .batch(vec![
                Operation::StreamInfo.request(json!({})),
                Operation::StreamInfo.request(json!({})),
            ])
            .await;
        assert!(
            response.is_ok(),
            "valid neighboring detail was discarded: {response:?}"
        );
        assert_eq!(
            response.unwrap()[1]["data"]["user"]["stream"]["id"],
            "valid-neighbor"
        );
    }

    #[tokio::test]
    async fn gql_does_not_exceed_five_inflight_requests() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/gql"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(10))
                    .set_body_json(json!({"data":{}})),
            )
            .mount(&server)
            .await;
        let http = Arc::new(http(&server));
        let client = TwitchClient::new(http.clone(), &session());
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..6 {
            let client = client.clone();
            tasks.spawn(async move { client.gql(json!({"query":"slow"})).await });
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            while server.received_requests().await.unwrap().len() < 5 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let inflight = server.received_requests().await.unwrap().len();
        http.cancel.cancel();
        while tasks.join_next().await.is_some() {}
        assert!(
            inflight <= 5,
            "{inflight} simultaneous requests reached the server before any completed"
        );
    }
}

use crate::config::Settings;

pub(crate) fn http(server: &MockServer) -> TwitchHttp {
    TwitchHttp::build(
        &Settings::default(),
        Some("testdevice"),
        CancellationToken::new(),
        Endpoints::mock(&server.uri()),
    )
    .unwrap()
}
pub(crate) fn session() -> Session {
    serde_json::from_value(json!({"version":1,"client_id":CLIENT_ID,"user_id":42,"device_id":"testdevice","access_token":"testtoken","refresh_token":"testrefresh"})).unwrap()
}
pub(crate) fn validation() -> serde_json::Value {
    json!({"client_id":CLIENT_ID,"user_id":"42","login":"miner","scopes":[],"expires_in":3600})
}

pub(crate) fn campaign_json(id: &str) -> serde_json::Value {
    let start = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    let end = (chrono::Utc::now() + chrono::Duration::hours(4)).to_rfc3339();
    json!({"id":id,"name":format!("Campaign {id}"),"game":{"id":"1","name":"Rust","slug":"rust"},
        "status":"ACTIVE","startAt":start,"endAt":end,"allow":{"isEnabled":false,"channels":[]},"self":{"isAccountConnected":true},
        "timeBasedDrops":[{"id":format!("drop-{id}"),"name":"Reward","startAt":start,"endAt":end,"requiredMinutesWatched":60,
            "preconditionDrops":[],"benefitEdges":[{"benefit":{"id":format!("benefit-{id}"),"name":"Hat","distributionType":"DIRECT_ENTITLEMENT","imageAssetURL":"https://static-cdn.jtvnw.net/hat.png"}}],
            "self":{"isClaimed":false,"currentMinutesWatched":12,"dropInstanceID":null}}]})
}

pub(crate) async fn gql_mock(
    server: &MockServer,
    handler: impl Fn(&serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
) {
    Mock::given(method("GET"))
        .and(path("/catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "lastUpdatedAt": chrono::Utc::now().to_rfc3339(), "data": []
        })))
        .with_priority(255)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/gql"))
        .respond_with(move |request: &wiremock::Request| {
            let request: serde_json::Value = request.body_json().unwrap();
            let response = if let Some(batch) = request.as_array() {
                serde_json::Value::Array(batch.iter().map(&handler).collect())
            } else {
                handler(&request)
            };
            ResponseTemplate::new(200).set_body_json(response)
        })
        .mount(server)
        .await;
}

#[tokio::test]
async fn gql_uses_smartbox_identity_rate_limit_and_preserves_nullable_neighbors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/gql"))
        .and(header("Client-Id", CLIENT_ID))
        .and(header("Authorization", "OAuth testtoken"))
        .and(header("Origin", CLIENT_ORIGIN))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"channels":[{"id":"1"},{"id":"2"}]},
            "errors":[{"message":"server error","path":["channels",0]}]})),
        )
        .mount(&server)
        .await;
    let client = TwitchClient::new(Arc::new(http(&server)), &session());
    let response = client.gql(json!({"query":"test"})).await.unwrap();
    assert!(response["data"]["channels"][0].is_null());
    assert_eq!(response["data"]["channels"][1]["id"], "2");
    assert_eq!(client.user_id, 42);
}

#[test]
fn gql_retries_only_recognized_failures_and_never_logs_upstream_secrets() {
    let mut temporary = json!({"errors":[{"message":"PersistedQueryNotFound"}]});
    assert!(gql_errors(&mut temporary, 0).unwrap());
    assert_eq!(gql_errors(&mut temporary, 1), Err(TwitchError::GraphQl));
    let mut unauthorized = json!({"errors":[{"message":"Unauthorized"}]});
    assert_eq!(
        gql_errors(&mut unauthorized, 0),
        Err(TwitchError::Unauthorized)
    );
    let mut hostile =
        json!({"errors":[{"message":"secret testtoken https://user:password@proxy/"}]});
    assert_eq!(
        gql_errors(&mut hostile, 0).unwrap_err().to_string(),
        "Twitch GraphQL request failed"
    );
    let mut invalid_path =
        json!({"data":{},"errors":[{"message":"server error","path":["missing"]}]});
    assert_eq!(gql_errors(&mut invalid_path, 0), Err(TwitchError::GraphQl));
}

#[tokio::test]
async fn public_discovery_rejections_are_http_errors_but_gql_auth_rejections_still_propagate() {
    for status in [401, 403] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tv"))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/gql"))
            .and(header("Authorization", "OAuth testtoken"))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;
        let mut http = http(&server);
        assert_eq!(
            http.discover_device().await,
            Err(TwitchError::Status(status))
        );
        let client = TwitchClient::new(Arc::new(http), &session());
        assert_eq!(client.gql(json!({})).await, Err(TwitchError::Unauthorized));
    }
}

#[tokio::test(start_paused = true)]
async fn gql_limiter_reserves_five_slots_per_second_and_cancels_waits() {
    let http = Arc::new(
        TwitchHttp::new(
            &Settings::default(),
            Some("device"),
            CancellationToken::new(),
        )
        .unwrap(),
    );
    for _ in 0..5 {
        http.acquire().await.unwrap();
    }
    let next = http.clone();
    let task = tokio::spawn(async move { next.acquire().await });
    tokio::task::yield_now().await;
    assert!(!task.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    task.await.unwrap().unwrap();
    for _ in 0..4 {
        http.acquire().await.unwrap();
    }
    let next = http.clone();
    let task = tokio::spawn(async move { next.acquire().await });
    tokio::task::yield_now().await;
    http.cancel.cancel();
    assert_eq!(task.await.unwrap(), Err(TwitchError::Cancelled));
}

#[tokio::test]
async fn idle_http_connections_expire_before_the_next_watch_minute() {
    use axum::{Router, extract::ConnectInfo, routing::get};
    use std::net::SocketAddr;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route(
        "/",
        get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.to_string() }),
    );
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let http = TwitchHttp::build(
        &Settings::default(),
        Some("testdevice"),
        CancellationToken::new(),
        Endpoints::mock(&format!("http://{address}")),
    )
    .unwrap();
    let fetch = || async {
        http.execute(
            http.request(Method::GET, http.endpoints.web.clone()),
            RetryPolicy::Never,
        )
        .await
        .unwrap()
        .into_body()
    };
    let first = fetch().await;
    let immediate = fetch().await;
    tokio::time::sleep(Duration::from_secs(16)).await;
    let after_idle = fetch().await;
    server.abort();
    let _ = server.await;
    assert_eq!(
        first, immediate,
        "nearby requests should still reuse connections"
    );
    assert_ne!(
        first, after_idle,
        "idle connections must expire after 15 seconds"
    );
}

#[tokio::test]
async fn cancellation_interrupts_inflight_http_without_retrying() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    let http = Arc::new(http(&server));
    let next = http.clone();
    let task = tokio::spawn(async move {
        next.execute(
            next.request(Method::GET, next.endpoints.tv.clone()),
            RetryPolicy::Replay,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    http.cancel.cancel();
    assert!(matches!(task.await.unwrap(), Err(TwitchError::Cancelled)));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

fn wire_response(status: u16, body: &[u8], missing: usize) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len() + missing).into_bytes();
    response.extend_from_slice(body);
    response
}

async fn body_server(
    responses: Vec<Vec<u8>>,
) -> (
    TwitchHttp,
    Arc<tokio::sync::Mutex<Vec<Vec<u8>>>>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let received = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = tokio::io::BufReader::new(stream);
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).await.unwrap();
            let mut requests = received.lock().await;
            let response = &responses[requests.len().min(responses.len() - 1)];
            requests.push(body);
            drop(requests);
            let mut stream = reader.into_inner();
            let _ = stream.write_all(response).await;
            // Used only by the cancellation-during-read test.
            if response
                .windows(b"Connection: keep-alive".len())
                .any(|v| v == b"Connection: keep-alive")
            {
                std::future::pending::<()>().await;
            }
            let _ = stream.shutdown().await;
        }
    });
    let http = TwitchHttp::build(
        &Settings::default(),
        Some("testdevice"),
        CancellationToken::new(),
        Endpoints::mock(&format!("http://{address}")),
    )
    .unwrap();
    (http, requests, server)
}

async fn stop_body_server(server: tokio::task::JoinHandle<()>) {
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn interrupted_read_only_responses_retry_with_the_identical_request() {
    for graphql in [false, true] {
        let (http, requests, server) = body_server(vec![
            wire_response(200, b"{", 100),
            wire_response(200, br#"{"data":{}}"#, 0),
        ])
        .await;
        let result = if graphql {
            let client = TwitchClient::new(Arc::new(http), &session());
            client
                .gql(super::operations::Operation::Inventory.request(json!({})))
                .await
                .map(|_| ())
        } else {
            http.execute(
                http.request(Method::GET, http.endpoints.web.clone()),
                RetryPolicy::Replay,
            )
            .await
            .map(|_| ())
        };
        let received = requests.lock().await.clone();
        stop_body_server(server).await;
        assert_eq!(result, Ok(()));
        assert_eq!(received.len(), 2);
        assert_eq!(received[0], received[1]);
    }
}

#[tokio::test]
async fn interrupted_response_retries_exhaust_after_five_attempts() {
    let (http, requests, server) = body_server(vec![wire_response(200, b"{", 100)]).await;
    let result = http
        .execute(
            http.request(Method::GET, http.endpoints.web.clone()),
            RetryPolicy::Replay,
        )
        .await;
    let count = requests.lock().await.len();
    stop_body_server(server).await;
    assert_eq!(result.unwrap_err(), TwitchError::Network);
    assert_eq!(count, 5);
}

#[tokio::test]
async fn interrupted_oauth_validation_rejections_still_refresh_the_saved_session() {
    for status in [401, 403] {
        let refreshed = serde_json::to_vec(
            &json!({"access_token":"newtoken","refresh_token":"newrefresh","expires_in":3600}),
        )
        .unwrap();
        let validated = serde_json::to_vec(&validation()).unwrap();
        let (http, requests, server) = body_server(vec![
            wire_response(status, b"{", 100),
            wire_response(200, &refreshed, 0),
            wire_response(200, &validated, 0),
        ])
        .await;
        let restored = session().restore(&http).await;
        let received = requests.lock().await.clone();
        stop_body_server(server).await;
        assert!(restored.is_ok(), "{status}: {:?}", restored.as_ref().err());
        let restored = restored.unwrap();
        assert_eq!(restored.access_token, "newtoken");
        assert_eq!(restored.user_id, 42);
        assert_eq!(
            received.len(),
            3,
            "validate, refresh, validate refreshed token"
        );
        let refresh = String::from_utf8(received[1].clone()).unwrap();
        assert!(
            refresh.contains("grant_type=refresh_token")
                && refresh.contains("refresh_token=testrefresh")
        );
    }
}

#[tokio::test]
async fn interrupted_successful_oauth_and_mutations_are_not_replayed() {
    use super::operations::Operation;
    let read = Operation::Inventory.request(json!({}));
    let mut raw_query = read.clone();
    raw_query["query"] = json!("mutation { something }");
    for operation in [
        Operation::ClaimDrop.request(json!({"input":{"dropInstanceID":"issued-instance"}})),
        Operation::DeleteNotification.request(json!({"input":{"id":"notification"}})),
        json!([read, Operation::DeleteNotification.request(json!({}))]),
        raw_query,
        json!({"operationName":"UnknownMutation"}),
    ] {
        let (http, requests, server) = body_server(vec![wire_response(200, b"{", 100)]).await;
        let client = TwitchClient::new(Arc::new(http), &session());
        let result = client.gql(operation).await;
        let count = requests.lock().await.len();
        stop_body_server(server).await;
        assert_eq!(result.unwrap_err(), TwitchError::Network);
        assert_eq!(count, 1);
    }
    let (http, requests, server) = body_server(vec![
        wire_response(200, br#"{"access_token":"synthetic"}"#, 100),
        wire_response(400, br#"{"message":"device code already consumed"}"#, 0),
    ])
    .await;
    let result = http.oauth_request(http::Request::builder().method(Method::POST)
        .uri("https://id.twitch.tv/oauth2/token").body(b"device_code=synthetic&grant_type=urn:ietf:params:oauth:grant-type:device_code".to_vec()).unwrap()).await;
    let count = requests.lock().await.len();
    stop_body_server(server).await;
    assert_eq!(result.unwrap_err(), TwitchError::Network);
    assert_eq!(
        count, 1,
        "a successful single-use exchange must not be replayed"
    );
}

#[tokio::test]
async fn response_replay_does_not_retry_disabled_invalid_or_acknowledged_responses() {
    let (http, requests, server) = body_server(vec![wire_response(200, b"{", 100)]).await;
    let result = http
        .execute(
            http.request(Method::GET, http.endpoints.web.clone()),
            RetryPolicy::Never,
        )
        .await;
    let count = requests.lock().await.len();
    stop_body_server(server).await;
    assert_eq!(result.unwrap_err(), TwitchError::Network);
    assert_eq!(count, 1);
    for body in [vec![b'{'], vec![b' '; super::MAX_BODY + 1]] {
        let (http, requests, server) = body_server(vec![wire_response(200, &body, 0)]).await;
        let client = TwitchClient::new(Arc::new(http), &session());
        let result = client
            .gql(super::operations::Operation::Inventory.request(json!({})))
            .await;
        let count = requests.lock().await.len();
        stop_body_server(server).await;
        assert_eq!(result, Err(TwitchError::InvalidResponse));
        assert_eq!(count, 1);
    }
    let (http, requests, server) = body_server(vec![wire_response(204, b"", 0)]).await;
    let mut channel = crate::domain::Channel::offline(
        crate::domain::ChannelIdentity {
            id: 10,
            login: "streamer".into(),
            name: "Streamer".into(),
        },
        false,
    );
    channel.broadcast_id = Some("broadcast".into());
    channel.beacon_url = Some(http.endpoints.web.clone());
    let client = TwitchClient::new(Arc::new(http), &session());
    let result = client.send_watch(&mut channel, chrono::Utc::now()).await;
    let count = requests.lock().await.len();
    stop_body_server(server).await;
    assert_eq!(result, Ok(true));
    assert_eq!(count, 1);
}

#[tokio::test]
async fn interrupted_error_bodies_preserve_authenticated_and_public_statuses() {
    for status in [400, 401, 403] {
        for endpoint in ["page", "gql", "oauth", "catalog", "games"] {
            let (http, requests, server) =
                body_server(vec![wire_response(status, b"{", 100)]).await;
            let result = if endpoint == "gql" {
                TwitchClient::new(Arc::new(http), &session())
                    .gql(super::operations::Operation::Inventory.request(json!({})))
                    .await
                    .map(|_| ())
            } else if endpoint == "games" {
                TwitchClient::new(Arc::new(http), &session())
                    .games(&crate::app::commands::GameQuery::Search("Rust".into()))
                    .await
                    .map(|_| ())
            } else if endpoint == "oauth" {
                http.oauth_request(
                    http::Request::builder()
                        .uri("https://id.twitch.tv/oauth2/validate")
                        .body(Vec::new())
                        .unwrap(),
                )
                .await
                .map(|_| ())
            } else if endpoint == "catalog" {
                http.catalog().await.map(|_| ())
            } else {
                http.execute(
                    http.request(Method::GET, http.endpoints.web.clone()),
                    RetryPolicy::Replay,
                )
                .await
                .map(|_| ())
            };
            let count = requests.lock().await.len();
            stop_body_server(server).await;
            let expected =
                if matches!(endpoint, "gql" | "oauth" | "games") && matches!(status, 401 | 403) {
                    TwitchError::Unauthorized
                } else {
                    TwitchError::Status(status)
                };
            assert_eq!(result, Err(expected));
            assert_eq!(count, 1);
        }
    }
}

#[tokio::test]
async fn cancellation_interrupts_response_read_and_retry_backoff() {
    for reading in [false, true] {
        let response = if reading {
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: keep-alive\r\n\r\n{".to_vec()
        } else {
            wire_response(200, b"{", 100)
        };
        let (http, requests, server) = body_server(vec![response]).await;
        let next = http.clone();
        let task = tokio::spawn(async move {
            next.execute(
                next.request(Method::GET, next.endpoints.web.clone()),
                RetryPolicy::Replay,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while requests.lock().await.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        http.cancel.cancel();
        let result = task.await.unwrap();
        let count = requests.lock().await.len();
        stop_body_server(server).await;
        assert_eq!(result.unwrap_err(), TwitchError::Cancelled);
        assert_eq!(count, 1);
    }
}

#[tokio::test]
async fn transport_refuses_redirects_with_credentials_and_normalizes_json_media_type() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/oauth2/validate"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/leak", server.uri())),
        )
        .mount(&server)
        .await;
    let http = http(&server);
    use twitch_oauth2::client::Client;
    let token = twitch_oauth2::AccessToken::new("testtoken".into());
    assert_eq!(
        http.req(token.validate_token_request())
            .await
            .unwrap()
            .status(),
        302
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
