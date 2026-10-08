use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::*;
use crate::{
    domain::ChannelIdentity,
    twitch::tests::{gql_mock, http, session},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn state() -> Playback {
    let mut channel = Channel::offline(
        ChannelIdentity {
            id: 10,
            login: "streamer".into(),
            name: "Streamer".into(),
        },
        false,
    );
    channel.broadcast_id = Some("broadcast".into());
    Playback::new(&channel).unwrap()
}

async fn playlist(server: &MockServer, url: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(url))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/vnd.apple.mpegurl")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

async fn setup(server: &MockServer) -> TwitchClient {
    gql_mock(server, |q| {
        assert_eq!(q["operationName"], "PlaybackAccessToken");
        assert_eq!(q["variables"], json!({"login":"streamer","isLive":true,"isVod":false,"vodID":"","platform":"web","playerType":"site"}));
        json!({"data":{"streamPlaybackAccessToken":{"value":"mock-token","signature":"mock-signature"}}})
    }).await;
    playlist(server, "/api/channel/hls/streamer.m3u8", "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100,CODECS=\"mp4a.40.2\"\n../../../media.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1000\n/high.m3u8\n").await;
    TwitchClient::new(Arc::new(http(server)), &session())
}

#[tokio::test]
async fn every_new_segment_is_checked_once_without_media_gets_or_account_headers() {
    let server = MockServer::start().await;
    let client = setup(&server).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let reads = calls.clone();
    Mock::given(method("GET"))
        .and(path("/media.m3u8"))
        .respond_with(move |_: &wiremock::Request| {
            let last = if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                "two"
            } else {
                "three"
            };
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/x-mpegURL; charset=utf-8")
                .set_body_string(format!(
                    "#EXTM3U\n#EXTINF:2,\n/one.ts\n#EXTINF:2,\n/{last}.ts\n"
                ))
        })
        .mount(&server)
        .await;
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let mut state = state();
    state.poll(&client).await.unwrap();
    state.poll(&client).await.unwrap();
    assert_eq!(state.seen.len(), 3);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.iter().filter(|r| r.method == "POST").count(), 1);
    assert_eq!(requests.iter().filter(|r| r.method == "HEAD").count(), 3);
    for request in requests.iter().filter(|r| r.url.path() != "/gql") {
        for name in [
            "authorization",
            "cookie",
            "client-id",
            "x-device-id",
            "client-session-id",
        ] {
            assert!(!request.headers.contains_key(name), "{name}");
        }
        assert!(request.method == "HEAD" || request.url.path().ends_with(".m3u8"));
        assert_ne!(request.url.path(), "/high.m3u8");
    }
}

#[tokio::test]
async fn failed_segments_retry_while_successful_segments_survive_playlist_renewal() {
    for status in [403, 404, 500] {
        let server = MockServer::start().await;
        let client = setup(&server).await;
        playlist(
            &server,
            "/media.m3u8",
            "#EXTM3U\n#EXTINF:2,\n/one.ts\n#EXTINF:2,\n/two.ts\n",
        )
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let failures = calls.clone();
        Mock::given(method("HEAD"))
            .and(path("/one.ts"))
            .respond_with(move |_: &wiremock::Request| {
                ResponseTemplate::new(if failures.fetch_add(1, Ordering::SeqCst) == 0 {
                    status
                } else {
                    200
                })
            })
            .mount(&server)
            .await;
        Mock::given(method("HEAD"))
            .and(path("/two.ts"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let mut state = state();
        assert_eq!(state.poll(&client).await, Err(TwitchError::Status(status)));
        assert_eq!(state.seen.len(), 1);
        assert_eq!(state.playlist.is_none(), status != 500);
        state.poll(&client).await.unwrap();
        assert_eq!(state.seen.len(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn expired_playlist_is_refetched_without_logging_out() {
    let server = MockServer::start().await;
    let client = setup(&server).await;
    Mock::given(method("GET"))
        .and(path("/media.m3u8"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let mut state = state();
    for _ in 0..2 {
        assert_eq!(state.poll(&client).await, Err(TwitchError::Status(401)));
        assert!(state.playlist.is_none());
    }
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        2
    );
}

#[tokio::test]
async fn slow_segments_time_out_without_losing_successful_checks_or_growing_the_cache() {
    let server = MockServer::start().await;
    let client = setup(&server).await;
    playlist(
        &server,
        "/media.m3u8",
        "#EXTM3U\n#EXTINF:2,\n/fast.ts\n#EXTINF:2,\n/slow.ts\n",
    )
    .await;
    Mock::given(method("HEAD"))
        .and(path("/fast.ts"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let reads = calls.clone();
    Mock::given(method("HEAD"))
        .and(path("/slow.ts"))
        .respond_with(move |_: &wiremock::Request| {
            if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(200).set_delay(Duration::from_secs(30))
            } else {
                ResponseTemplate::new(200)
            }
        })
        .mount(&server)
        .await;
    let mut state = state();
    state.seen = (0..MAX_SEGMENTS)
        .map(|n| format!("{}/old-{n}.ts", server.uri()).parse().unwrap())
        .collect();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(8), state.poll(&client))
            .await
            .unwrap(),
        Err(TwitchError::Network)
    );
    assert_eq!(state.seen.len(), MAX_SEGMENTS);
    assert_eq!(state.seen.back().unwrap().path(), "/fast.ts");
    state.poll(&client).await.unwrap();
    assert_eq!(state.seen.len(), MAX_SEGMENTS);
    assert_eq!(state.seen.back().unwrap().path(), "/slow.ts");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn malformed_or_untrusted_playlists_never_fetch_media() {
    let server = MockServer::start().await;
    let client = setup(&server).await;
    let base: Url = format!("{}/master.m3u8", server.uri()).parse().unwrap();
    for body in [
        "not a playlist",
        "#EXTM3U\n/segment.ts",
        "#EXTM3U\n#EXTINF:2,\n/segment.ts",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=10\n/segment.ts",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=10\n",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=10\nhttps://evil.test/a.m3u8",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=10\nhttps://twitch.tv.evil.test/a.m3u8",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=10\nhttp://127.0.0.1/a.m3u8",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=10\nhttps://user@video.ttvnw.net/a.m3u8",
    ] {
        assert!(
            playlist_urls(body, &base, true, &client.http).is_err(),
            "{body}"
        );
    }
    for raw in [
        "https://video.ttvnw.net/a.ts#fragment",
        "https://video.ttvnw.net:8443/a.ts",
        "https://127.0.0.1/a.ts",
        "file:///a.ts",
    ] {
        assert!(!client.http.stream_url(&Url::parse(raw).unwrap()));
    }
    assert!(
        playlist_urls(
            "#EXTM3U\n#EXTINF:2,\nhttps://video.ttvnw.net/a.ts",
            &base,
            false,
            &client.http
        )
        .is_ok()
    );
    assert!(
        playlist_urls(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=10\n/a.m3u8",
            &base,
            false,
            &client.http
        )
        .is_err()
    );
    assert!(
        playlist_urls(
            &format!(
                "#EXTM3U\n{}",
                "#EXTINF:2,\n/a.ts\n".repeat(MAX_SEGMENTS + 1)
            ),
            &base,
            false,
            &client.http
        )
        .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn response_types_redirects_and_body_limits_fail_closed() {
    for (status, content_type, body) in [
        (302, "text/plain", String::new()),
        (200, "video/mp2t", "binary".into()),
        (
            200,
            "application/vnd.apple.mpegurl",
            "x".repeat(MAX_PLAYLIST + 1),
        ),
    ] {
        let server = MockServer::start().await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        Mock::given(method("GET"))
            .and(path("/media.m3u8"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_raw(body, content_type)
                    .insert_header("Location", format!("{}/segment.ts", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            client
                .http
                .stream_request(
                    Method::GET,
                    format!("{}/media.m3u8", server.uri()).parse().unwrap()
                )
                .await
                .is_err(),
            "status={status}, type={content_type}"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn authenticated_token_failures_propagate_and_cancellation_stops_requests() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let client = TwitchClient::new(Arc::new(http(&server)), &session());
    assert_eq!(state().poll(&client).await, Err(TwitchError::Unauthorized));
    server.reset().await;
    client.http.cancel.cancel();
    assert_eq!(state().poll(&client).await, Err(TwitchError::Cancelled));
    assert!(server.received_requests().await.unwrap().is_empty());
}
