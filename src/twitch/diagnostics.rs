//! Server-only diagnostics. Advanced captures require an explicit logging target opt-in.
use std::{
    error::Error,
    sync::{
        LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::TwitchError;

const CAPTURE_LIMIT: usize = 16 * 1024;
const REDACTED: &str = "[redacted]";

pub(super) fn enabled() -> bool {
    tracing::enabled!(target: "tdm_diagnostics", tracing::Level::DEBUG)
}

pub(super) fn request_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Request-local credential inventory; never Debug-format this struct or a request.
#[derive(Clone, Default)]
pub(super) struct Capture {
    secrets: Vec<String>,
    complete: bool,
}

fn sensitive(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect();
    [
        "token",
        "secret",
        "password",
        "cookie",
        "authorization",
        "credential",
    ]
    .iter()
    .any(|part| key.contains(part))
        || key.ends_with("deviceid")
        || matches!(
            key.as_str(),
            "devicecode"
                | "usercode"
                | "deviceid"
                | "sessionid"
                | "clientsessionid"
                | "dropinstanceid"
                | "claimid"
        )
}

impl Capture {
    pub(super) fn new(proxy: &str, device: &str) -> Self {
        let mut capture = Self {
            complete: true,
            ..Self::default()
        };
        if let Ok(proxy) = url::Url::parse(proxy) {
            capture.secret(proxy.username());
            if let Some(password) = proxy.password() {
                capture.secret(password);
            }
            // Decode URL-encoded user information without logging either representation.
            capture.secret(
                &percent_encoding::percent_decode_str(proxy.username()).decode_utf8_lossy(),
            );
            if let Some(password) = proxy.password() {
                capture.secret(&percent_encoding::percent_decode_str(password).decode_utf8_lossy());
            }
        }
        capture.secret(device);
        capture
    }

    fn secret(&mut self, value: &str) {
        if !self.complete || value.is_empty() || self.secrets.iter().any(|secret| secret == value) {
            return;
        }
        if self.secrets.len() >= 256
            || value.len() + self.secrets.iter().map(String::len).sum::<usize>() > 32 * 1024
        {
            self.complete = false;
            return;
        }
        self.secrets.push(value.to_owned());
    }

    fn values(&mut self, value: &Value, private: bool) {
        match value {
            Value::String(value) if private => self.secret(value),
            Value::Number(value) if private => self.secret(&value.to_string()),
            Value::Array(values) => {
                for value in values {
                    self.values(value, private);
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    self.values(value, private || sensitive(key));
                }
            }
            _ => {}
        }
    }

    pub(super) fn request(
        &self,
        request: &reqwest::Request,
        cookies: Option<http::HeaderValue>,
    ) -> Self {
        let mut capture = self.clone();
        if !enabled() {
            return capture;
        }
        for (key, value) in request.url().query_pairs() {
            if sensitive(&key) {
                capture.secret(&value);
            }
        }
        for (key, value) in request.headers() {
            if sensitive(key.as_str())
                && let Ok(value) = value.to_str()
            {
                capture.secret(value);
                if let Some((_, credential)) = value.split_once(' ') {
                    capture.secret(credential);
                }
            }
        }
        for value in cookies
            .iter()
            .chain(request.headers().get_all(http::header::COOKIE).iter())
        {
            if let Ok(value) = value.to_str() {
                capture.secret(value);
                for cookie in cookie::Cookie::split_parse(value).flatten() {
                    capture.secret(cookie.value());
                    capture.secret(cookie.value_trimmed());
                }
            }
        }
        if let Some(body) = request.body().and_then(reqwest::Body::as_bytes) {
            if let Ok(body) = serde_json::from_slice::<Value>(body) {
                capture.values(&body, false);
            } else {
                for (key, value) in url::form_urlencoded::parse(body) {
                    if sensitive(&key) {
                        capture.secret(&value);
                    }
                }
            }
        }
        capture
    }

    fn text(&self, text: &str) -> String {
        if !self.complete {
            return "[withheld: credential redaction limit reached]".into();
        }
        // Withhold oversized strings whole: truncating input could expose part of a secret.
        if text.len() > CAPTURE_LIMIT {
            return "[oversized text withheld]".into();
        }
        static URLS: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"(?i)\b[a-z][a-z0-9+.-]*://[^\s<>\"']+"#).unwrap());
        static AUTH: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"(?i)\b(?:bearer|oauth|basic)\s+[^\s,;\"']+"#).unwrap());
        static OPAQUE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"[A-Za-z0-9_+/=-]{24,}").unwrap());
        static CREDENTIAL_TEXT: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
            r#"(?i)\b[a-z0-9_-]*(?:token|secret|password|cookie|authorization|credential|device[_-]?code|user[_-]?code)[a-z0-9_-]*[\s\\\"']*[:=]"#
        ).unwrap()
        });
        // A string can itself contain encoded JSON/HTML or credential assignments.
        // Without a reliable structure, withhold that string rather than guess its boundary.
        if CREDENTIAL_TEXT.is_match(text) {
            return "[credential-bearing text withheld]".into();
        }
        // Mark original bytes first: a secret may itself be a URL/auth delimiter.
        // A bounded mask merges overlapping credentials without rewriting inserted markers.
        let mut private = vec![false; text.len()];
        for pattern in [&*URLS, &*AUTH, &*OPAQUE] {
            for found in pattern.find_iter(text) {
                private[found.range()].fill(true);
            }
        }
        for (index, _) in text.char_indices() {
            if let Some(length) = self
                .secrets
                .iter()
                .filter(|secret| text[index..].starts_with(secret.as_str()))
                .map(String::len)
                .max()
            {
                private[index..index + length].fill(true);
            }
        }
        let mut output = String::new();
        for (index, ch) in text.char_indices() {
            if !private[index] {
                output.push(ch);
            } else if index == 0 || !private[index - 1] {
                output.push_str(REDACTED);
            }
            if output.len() >= CAPTURE_LIMIT {
                output.push_str(" [truncated]");
                break;
            }
        }
        bounded(output, CAPTURE_LIMIT)
    }

    fn json(&self, value: &Value, budget: &mut usize, depth: usize) -> Value {
        if *budget == 0 || depth > 12 {
            return Value::String("[capture limit reached]".into());
        }
        *budget -= 1;
        match value {
            Value::String(value) => Value::String(bounded(self.text(value), 1024)),
            Value::Number(value) if self.secrets.contains(&value.to_string()) => {
                Value::String(REDACTED.into())
            }
            Value::Array(values) => {
                let mut output: Vec<_> = values
                    .iter()
                    .take(32)
                    .map(|value| self.json(value, budget, depth + 1))
                    .collect();
                if values.len() > 32 {
                    output.push(Value::String(format!(
                        "[{} entries omitted]",
                        values.len() - 32
                    )));
                }
                Value::Array(output)
            }
            Value::Object(values) => {
                let mut output = serde_json::Map::new();
                for (key, value) in values.iter().take(64) {
                    output.insert(
                        bounded(self.text(key), 128),
                        if sensitive(key) {
                            Value::String(REDACTED.into())
                        } else {
                            self.json(value, budget, depth + 1)
                        },
                    );
                }
                if values.len() > 64 {
                    output.insert("[omitted fields]".into(), (values.len() - 64).into());
                }
                Value::Object(output)
            }
            _ => value.clone(),
        }
    }

    pub(super) fn response(
        &self,
        response: &http::Response<Vec<u8>>,
        attempt: usize,
        elapsed_ms: u128,
    ) {
        if !enabled() {
            return;
        }
        let mut capture = self.clone();
        for (key, value) in response.headers() {
            if sensitive(key.as_str())
                && let Ok(value) = value.to_str()
            {
                capture.secret(value);
                if let Some((_, credential)) = value.split_once(' ') {
                    capture.secret(credential);
                }
            }
        }
        for value in response.headers().get_all(http::header::SET_COOKIE) {
            if let Ok(value) = value.to_str()
                && let Ok(cookie) = cookie::Cookie::parse(value)
            {
                capture.secret(cookie.value());
                capture.secret(cookie.value_trimmed());
            }
        }
        let body = response.body();
        let preview = match serde_json::from_slice::<Value>(body) {
            Ok(value) => {
                capture.values(&value, false);
                if capture.complete {
                    bounded(capture.json(&value, &mut 256, 0).to_string(), CAPTURE_LIMIT)
                } else {
                    "[withheld: credential redaction limit reached]".into()
                }
            }
            // Arbitrary HTML/binary/malformed JSON may contain new credentials that cannot
            // be identified structurally. Preserve evidence without writing raw frames.
            Err(_) => {
                "[non-JSON body withheld; see size, fingerprint and parser diagnostics]".into()
            }
        };
        let mut headers = serde_json::Map::new();
        for name in [
            "content-type",
            "content-length",
            "retry-after",
            "x-request-id",
            "x-amzn-requestid",
            "x-amz-cf-id",
            "cf-ray",
        ] {
            if let Some(value) = response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
            {
                headers.insert(
                    name.into(),
                    Value::String(bounded(capture.text(value), 256)),
                );
            }
        }
        tracing::debug!(target: "tdm_diagnostics", attempt, elapsed_ms,
            status = response.status().as_u16(), http_version = ?response.version(), bytes = body.len(),
            fingerprint = %hex::encode(Sha256::digest(body)), headers = %serde_json::Value::Object(headers),
            response = %preview, "Upstream response diagnostic (redacted, bounded)");
    }

    pub(super) fn network(
        &self,
        error: &reqwest::Error,
        stage: &'static str,
        attempt: usize,
        elapsed_ms: u128,
    ) {
        network(error, stage, attempt);
        if !enabled() {
            return;
        }
        let mut chain = Vec::new();
        let mut current: Option<&(dyn Error + 'static)> = Some(error);
        while let Some(cause) = current {
            if chain.len() == 12 {
                chain.push("[additional causes omitted]".into());
                break;
            }
            chain.push(bounded(self.text(&cause.to_string()), 1024));
            current = cause.source();
        }
        tracing::debug!(target: "tdm_diagnostics", stage, attempt, elapsed_ms, causes = ?chain,
            "Upstream transport error chain (redacted, bounded)");
    }
}

fn bounded(mut text: String, limit: usize) -> String {
    if text.len() > limit {
        const MARKER: &str = " [truncated]";
        let mut boundary = limit.saturating_sub(MARKER.len());
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
        text.push_str(MARKER);
    }
    text
}

pub(super) fn kind(value: Option<&Value>) -> &'static str {
    match value {
        None => "missing",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

pub(super) fn invalid(
    operation: &'static str,
    reason: &'static str,
    value: Option<&Value>,
) -> TwitchError {
    tracing::warn!(
        operation,
        reason,
        actual_type = kind(value),
        "Invalid upstream response"
    );
    TwitchError::InvalidResponse
}

pub(super) fn json(body: &[u8], status: u16) -> Result<Value, TwitchError> {
    serde_json::from_slice(body).map_err(|error| {
        // Transitive serde_json features can also produce Data errors (e.g. its
        // reserved RawValue key). Their Display may echo an upstream value.
        let reason = if error.is_syntax() || error.is_eof() {
            error.to_string()
        } else {
            "JSON value decoding failed (details withheld)".to_owned()
        };
        tracing::warn!(
            status, bytes = body.len(), category = ?error.classify(),
            line = error.line(), column = error.column(), %reason,
            "Upstream response is not valid JSON"
        );
        TwitchError::InvalidResponse
    })
}

pub(super) fn operation(request: &Value) -> &'static str {
    // Request variables and arbitrary operation names must never enter logs.
    match request["operationName"].as_str().unwrap_or_default() {
        "Inventory" => "Inventory",
        "AccountProfile" => "AccountProfile",
        "AccountBadges" => "AccountBadges",
        "DirectoryPage_Game" => "DirectoryPage_Game",
        "VideoPlayerStreamInfoOverlayChannel" => "VideoPlayerStreamInfoOverlayChannel",
        "DropCurrentSessionContext" => "DropCurrentSessionContext",
        "PlaybackAccessToken" => "PlaybackAccessToken",
        "DropsPage_ClaimDropRewards" => "DropsPage_ClaimDropRewards",
        "DropsHighlightService_AvailableDrops" => "DropsHighlightService_AvailableDrops",
        "OnsiteNotifications_DeleteNotification" => "OnsiteNotifications_DeleteNotification",
        _ if request.is_array() => "batch",
        _ => "unknown",
    }
}

pub(super) fn graphql(response: &Value, operation: &'static str, attempt: u32, index: usize) {
    let errors = response["errors"].as_array();
    if response.get("error").is_some() || errors.is_some_and(|v| !v.is_empty()) {
        let messages: Vec<_> = errors
            .into_iter()
            .flatten()
            .take(4)
            .map(
                |error| match error["message"].as_str().unwrap_or_default() {
                    message @ ("Unauthorized"
                    | "unauthorized"
                    | "authentication required"
                    | "invalid oauth token"
                    | "service error"
                    | "PersistedQueryNotFound"
                    | "service timeout"
                    | "service unavailable"
                    | "context deadline exceeded"
                    | "server error") => message,
                    _ => "unrecognized (withheld)",
                },
            )
            .collect();
        // Correlate repeated unknown errors without retaining arbitrary text that
        // could echo credentials. Cap output, even for a hostile errors array.
        let mut fingerprint = Sha256::new();
        fingerprint.update(response["error"].to_string());
        fingerprint.update(response["errors"].to_string());
        let fingerprint = hex::encode(fingerprint.finalize());
        tracing::warn!(operation, attempt = attempt + 1, batch_index = index,
            error_count = errors.map_or(0, Vec::len), ?messages, %fingerprint,
            top_level_error = response.get("error").is_some(), "GraphQL response contains errors");
    }
}

pub(super) fn network(error: &reqwest::Error, stage: &'static str, attempt: usize) {
    // Error Display/source Display can contain proxy credentials and request URLs.
    // Preserve typed transport causes instead of attempting blacklist redaction.
    let mut source = std::error::Error::source(error);
    let mut io_kind = None;
    let mut os_code = None;
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            io_kind = Some(io.kind());
            os_code = io.raw_os_error();
        }
        source = cause.source();
    }
    tracing::warn!(
        stage,
        attempt,
        timeout = error.is_timeout(),
        connect = error.is_connect(),
        body = error.is_body(),
        decode = error.is_decode(),
        request = error.is_request(),
        ?io_kind,
        ?os_code,
        "Upstream transport failed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twitch::{
        RetryPolicy, TwitchClient,
        operations::Operation,
        tests::{http, session},
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tracing::instrument::WithSubscriber;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[derive(Clone, Default)]
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Writer {
        fn register_test_dispatch() {
            // tracing-core's single-dispatch cache uses the registering thread's default.
            // Keep a second, quiet dispatch alive so unrelated parallel tests cannot cache
            // NEVER for callsites first visited outside this test's temporary scope.
            static QUIET: LazyLock<tracing::Dispatch> = LazyLock::new(|| {
                tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default())
            });
            LazyLock::force(&QUIET);
        }
        fn advanced_subscriber(&self) -> impl tracing::Subscriber {
            Self::register_test_dispatch();
            let writer = self.clone();
            tracing_subscriber::fmt()
                .with_env_filter("info,tdm_diagnostics=debug")
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .finish()
        }
        fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
            Self::register_test_dispatch();
            let writer = self.clone();
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .finish()
        }
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    #[tokio::test]
    async fn advanced_responses_include_unknown_errors_and_retries_but_redact_credentials() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let server = MockServer::start().await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let counted = attempts.clone();
        Mock::given(method("POST")).respond_with(move |_: &wiremock::Request| {
            let status = if counted.fetch_add(1, Ordering::SeqCst) == 0 { 500 } else { 200 };
            let response = ResponseTemplate::new(status);
            let response = if status == 500 { response.insert_header("Set-Cookie", "session=server-cookie; HttpOnly") } else { response };
            response
                .insert_header("X-Request-Id", "request-42")
                .set_body_json(json!({
                    "errors": [{"message": "New Twitch failure: testtoken, request-cookie, server-cookie, device-secret, new-access, refresh-secret", "extensions": {"code": "NEW_FAILURE"}}],
                    "access_token": "new-access", "nested": {"refreshToken": "refresh-secret"},
                    "device_code": "device-secret", "user_code": 123456, "numeric_echo": 123456, "minutes": 17,
                    "url": "https://private-user:private-pass@host.invalid/path?token=private-query",
                    "nested_text": "response: {\\\"password\\\": \\\"hidden-pass\\\"}"
                }))
        }).mount(&server).await;
        let http = http(&server);
        let output = Writer::default();
        let response = http
            .execute(
                http.client
                    .post(server.uri())
                    .header("Authorization", "OAuth testtoken")
                    .header("Cookie", "session=request-cookie")
                    .form(&[("device_code", "device-secret")]),
                RetryPolicy::Transport,
            )
            .with_subscriber(output.advanced_subscriber())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        let text = output.text();
        assert!(
            text.contains("New Twitch failure") && text.contains("NEW_FAILURE"),
            "{text}"
        );
        assert!(
            text.contains("status=500")
                && text.contains("status=200")
                && text.contains("request-42")
        );
        assert!(
            text.contains("diagnostic_id=")
                && text.contains("elapsed_ms=")
                && text.contains("fingerprint=")
        );
        for secret in [
            "testtoken",
            "request-cookie",
            "server-cookie",
            "device-secret",
            "new-access",
            "refresh-secret",
            "private-user",
            "private-pass",
            "private-query",
            "hidden-pass",
            "123456",
        ] {
            assert!(!text.contains(secret), "leaked {secret}: {text}");
        }
        // Capturing does not alter the response handed to application parsers.
        assert!(
            String::from_utf8(response.into_body())
                .unwrap()
                .contains("new-access")
        );
    }

    #[tokio::test]
    async fn advanced_transport_keeps_nested_request_failure_cause_without_urls_or_tokens() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            socket.readable().await.unwrap();
            let _ = socket.try_read(&mut [0; 8192]);
            // Close without any HTTP response, reproducing a send/request error.
        });
        let server = MockServer::start().await;
        let http = http(&server);
        let output = Writer::default();
        let error = http
            .execute(
                http.client
                    .post(format!(
                        "http://{address}/private-path?token=private-secret"
                    ))
                    .header("Authorization", "OAuth testtoken"),
                RetryPolicy::Never,
            )
            .with_subscriber(output.advanced_subscriber())
            .await
            .unwrap_err();
        peer.await.unwrap();
        assert_eq!(error, TwitchError::Network);
        let text = output.text();
        assert!(
            text.contains("transport error chain")
                && text.contains("causes=[")
                && text.contains("connection"),
            "{text}"
        );
        assert!(
            !text.contains("private-")
                && !text.contains("testtoken")
                && !text.contains(&address.to_string()),
            "{text}"
        );
    }

    #[tokio::test]
    async fn advanced_retry_capture_has_a_deadline_and_remains_cancellable() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for cancel in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (ready, received) = tokio::sync::oneshot::channel();
            let peer = tokio::spawn(async move {
                let (mut first, _) = listener.accept().await.unwrap();
                assert!(first.read(&mut [0; 8192]).await.unwrap() > 0);
                first
                    .write_all(
                        b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 1000\r\n\r\n",
                    )
                    .await
                    .unwrap();
                let _ = ready.send(());
                // Leave the first body stalled while accepting the retry on a fresh connection.
                let (mut second, _) = listener.accept().await.unwrap();
                assert!(second.read(&mut [0; 8192]).await.unwrap() > 0);
                second
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .await
                    .unwrap();
            });
            let server = MockServer::start().await;
            let http = http(&server);
            let output = Writer::default();
            let work = http
                .execute(
                    http.client.post(format!("http://{address}/")),
                    RetryPolicy::Transport,
                )
                .with_subscriber(output.advanced_subscriber());
            let cancellation = async {
                received.await.unwrap();
                if cancel {
                    http.cancel.cancel();
                }
            };
            let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                tokio::join!(work, cancellation)
            })
            .await
            .expect("capture must not hang retry or cancellation");
            if cancel {
                assert_eq!(result.unwrap_err(), TwitchError::Cancelled);
                peer.abort();
                let _ = peer.await;
            } else {
                assert_eq!(result.unwrap().status(), 200);
                assert!(
                    output.text().contains("one-second budget"),
                    "{}",
                    output.text()
                );
                peer.await.unwrap();
            }
        }
    }

    #[test]
    fn advanced_scoped_capture_survives_uninstrumented_callsite_registration() {
        let output = Writer::default();
        let dispatch = tracing::Dispatch::new(output.advanced_subscriber());
        let capture = Capture::new("", "");
        let response = http::Response::builder()
            .body(b"{\"message\":\"Visible diagnostic\"}".to_vec())
            .unwrap();
        assert!(!enabled());
        capture.response(&response, 1, 0);
        tracing::dispatcher::with_default(&dispatch, || {
            assert!(enabled());
            capture.response(&response, 1, 0);
        });
        assert!(output.text().contains("Visible diagnostic"));
    }

    #[test]
    fn advanced_redacts_decoded_proxy_and_quoted_cookie_echoes() {
        let output = Writer::default();
        tracing::subscriber::with_default(output.advanced_subscriber(), || {
            let capture = Capture::new("http://u+ser:&p%40ssword+suffix@proxy.invalid", "");
            assert_eq!(
                capture.text("u+ser &p@ssword+suffix"),
                "[redacted] [redacted]"
            );
            let request = reqwest::Client::new()
                .get("http://example.invalid")
                .header("Cookie", "session=\"request-cookie\"")
                .build()
                .unwrap();
            let capture = capture.request(
                &request,
                Some(http::HeaderValue::from_static("jar=\"jar-cookie\"")),
            );
            let response = http::Response::builder()
                .header("Set-Cookie", "session=\"short-cookie\"; HttpOnly")
                .body(serde_json::to_vec(&json!({"message": "&p@ssword+suffix short-cookie request-cookie jar-cookie"})).unwrap()).unwrap();
            capture.response(&response, 1, 0);
        });
        let text = output.text();
        assert!(text.contains("Upstream response diagnostic"));
        for secret in ["p@ssword", "short-cookie", "request-cookie", "jar-cookie"] {
            assert!(!text.contains(secret), "{text}");
        }
    }

    #[test]
    fn advanced_redaction_never_rescans_markers_or_expands_oversized_text() {
        let mut capture = Capture::new("", "");
        for secret in ["v", "r", "e", "d", "a", "c", "t", "[", "]"] {
            capture.secret(secret);
        }
        assert_eq!(capture.text("v"), REDACTED);
        assert!(capture.text(&"v".repeat(CAPTURE_LIMIT)).len() < CAPTURE_LIMIT + 32);
        assert_eq!(
            capture.text(&"v".repeat(16 * 1024 * 1024)),
            "[oversized text withheld]"
        );
        assert_eq!(capture.text("🦀v"), "🦀[redacted]");
        for secret in [":", "/", " ", "Bearer", "https"] {
            let mut capture = Capture::new("", "");
            capture.secret(secret);
            for text in [
                "https://private-user:private-pass@host.invalid/private-secret",
                "Bearer short-secret",
            ] {
                assert_eq!(capture.text(text), REDACTED);
            }
        }
        let mut overlapping = Capture::new("", "");
        overlapping.secret("aba");
        overlapping.secret("bab");
        assert_eq!(overlapping.text("ababa"), REDACTED);
    }

    #[test]
    fn advanced_capture_limits_and_non_json_fail_closed_after_redaction() {
        let output = Writer::default();
        tracing::subscriber::with_default(output.advanced_subscriber(), || {
            let capture = Capture::new(
                "http://proxy-user:p%40ssword@proxy.invalid",
                "device-secret",
            );
            assert!(
                !capture
                    .text("proxy-user p@ssword p%40ssword device-secret")
                    .contains("ssword")
            );
            let response = http::Response::builder().status(200).body(
                serde_json::to_vec(&json!({"message": "Readable failure ".repeat(400), "accessToken": "secret-value", "echo": "secret-value", "rows": vec![json!({"value": 1}); 2000]})).unwrap()).unwrap();
            capture.response(&response, 1, 12);
            let invalid = http::Response::builder()
                .status(502)
                .body(b"<html>access_token=private-secret</html>".to_vec())
                .unwrap();
            capture.response(&invalid, 1, 15);
            let mut too_many = capture.clone();
            for index in 0..300 {
                too_many.secret(&format!("private-{index}"));
            }
            too_many.response(&response, 1, 16);
        });
        let text = output.text();
        assert!(
            text.contains("Readable failure")
                && text.contains("truncated")
                && text.contains("entries omitted"),
            "{text}"
        );
        assert!(
            text.contains("non-JSON body withheld")
                && text.contains("credential redaction limit reached")
        );
        assert!(
            !text.contains("private-secret")
                && !text.contains("secret-value")
                && !text.contains("<html>")
        );
        assert!(
            text.len() < 3 * (CAPTURE_LIMIT + 2048),
            "{} bytes",
            text.len()
        );
    }

    #[tokio::test]
    async fn malformed_progress_logs_field_and_type_without_exposing_values_or_changing_public_error()
     {
        for (mut drop, expected) in [
            (
                json!({"dropID":"private-id", "currentMinutesWatched":null}),
                "currentMinutesWatched must fit u32",
            ),
            (
                json!({"dropID":"", "currentMinutesWatched":2}),
                "dropID must be a nonempty string",
            ),
            (
                json!({"dropID":"private-id", "currentMinutesWatched":"private-secret"}),
                "currentMinutesWatched must fit u32",
            ),
        ] {
            drop["channel"] = json!({"id":"123456"});
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/gql"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"data":{"currentUser":{"dropCurrentSession":drop}}})),
                )
                .mount(&server)
                .await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            let output = Writer::default();
            let error = client
                .current_drop(123456)
                .with_subscriber(output.subscriber())
                .await
                .unwrap_err();
            assert_eq!(error.to_string(), "Twitch returned an invalid response");
            let text = output.text();
            assert!(
                text.contains("CurrentDrop") && text.contains(expected),
                "{text}"
            );
            assert!(
                !text.contains("private-")
                    && !text.contains("testtoken")
                    && !text.contains("123456")
            );
        }
    }

    #[tokio::test]
    async fn json_and_graphql_failures_include_operation_and_safe_details() {
        for (body, expected) in [
            (
                "<html>private-secret</html>",
                "expected value at line 1 column 1",
            ),
            (
                r#"{"errors":[{"message":"PersistedQueryNotFound"}]}"#,
                "PersistedQueryNotFound",
            ),
            (
                r#"{"errors":[{"message":"private-secret https://user:password@proxy/?token=testtoken"}]}"#,
                "unrecognized (withheld)",
            ),
            ("null", "expected response object"),
            (
                r#"{"$serde_json::private::RawValue":987654321}"#,
                "JSON value decoding failed (details withheld)",
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/gql"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&server)
                .await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            let output = Writer::default();
            assert!(
                client
                    .gql(Operation::CurrentDrop.request(json!({"channelID":"private-variable"})))
                    .with_subscriber(output.subscriber())
                    .await
                    .is_err()
            );
            let text = output.text();
            assert!(
                text.contains("DropCurrentSessionContext") && text.contains(expected),
                "{text}"
            );
            for secret in [
                "private-",
                "password",
                "testtoken",
                "<html>",
                "987654321",
                "RawValue",
            ] {
                assert!(!text.contains(secret), "{text}");
            }
        }
    }

    #[test]
    fn diagnostics_are_bounded_and_do_not_echo_arbitrary_keys_paths_or_error_text() {
        let output = Writer::default();
        let hostile = json!({"errors": (0..1000).map(|_| json!({
            "message":"private-secret", "path":["private-path"], "extensions":{"private-key":"private-value"}
        })).collect::<Vec<_>>()});
        tracing::subscriber::with_default(output.subscriber(), || {
            graphql(&hostile, "Inventory", 0, 0);
            invalid("CurrentDrop", "expected string", Some(&hostile));
        });
        let text = output.text();
        assert!(text.contains("error_count=1000") && text.contains("fingerprint="));
        assert!(!text.contains("private-") && text.len() < 1500);
        assert_eq!(
            operation(&json!({"operationName":"private-operation"})),
            "unknown"
        );
    }

    #[tokio::test]
    async fn playback_token_response_is_redacted_as_a_whole() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/gql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{
                "streamPlaybackAccessToken":{"value":"private-playback-token","signature":"private-playback-signature"}
            }})))
            .mount(&server).await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let output = Writer::default();
        client
            .gql(Operation::PlaybackAccessToken.request(json!({"login":"streamer"})))
            .with_subscriber(output.advanced_subscriber())
            .await
            .unwrap();
        let text = output.text();
        assert!(
            text.contains("PlaybackAccessToken") && text.contains("[redacted]"),
            "{text}"
        );
        assert!(
            !text.contains("private-playback-") && !text.contains("testtoken"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn partial_catalog_logs_rejection_reason_without_exposing_account_records() {
        for stale in [true, false] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/gql"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"data":{"currentUser":{"inventory":{
                        "dropCampaignsInProgress":[{"id":"private-id"}], "gameEventDrops":[]
                    }}}}),
                ))
                .mount(&server)
                .await;
            Mock::given(method("GET")).and(path("/catalog"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "lastUpdatedAt": (chrono::Utc::now() - chrono::Duration::minutes(if stale {31} else {0})).to_rfc3339(),
                    "data":[{"rewards":[null]}]
                }))).mount(&server).await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            let output = Writer::default();
            let inventory = client
                .inventory()
                .with_subscriber(output.subscriber())
                .await
                .unwrap();
            assert!(!inventory.status.available);
            let text = output.text();
            assert!(
                text.contains("Account inventory is partial")
                    && text.contains("malformed_records=1"),
                "{text}"
            );
            assert!(
                text.contains(if stale {
                    "outside the freshness window"
                } else {
                    "invalid_records=1"
                }),
                "{text}"
            );
            assert!(!text.contains("private-id") && !text.contains("testtoken"));
        }
    }

    #[tokio::test]
    async fn batch_diagnostics_are_capped_and_preserve_partial_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/gql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![
                json!({
                    "data":{"user":null}, "errors":[{"message":"server error", "path":["user"]}]
                });
                20
            ]))
            .mount(&server)
            .await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let output = Writer::default();
        let result = client
            .batch(vec![Operation::StreamInfo.request(json!({})); 20])
            .with_subscriber(output.subscriber())
            .await
            .unwrap();
        assert_eq!(result.len(), 20);
        let text = output.text();
        assert_eq!(
            text.matches("GraphQL response contains errors").count(),
            4,
            "{text}"
        );
        assert!(
            text.contains("omitted=16") && text.contains("VideoPlayerStreamInfoOverlayChannel"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn http_and_transport_failures_log_status_or_typed_cause_without_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("Set-Cookie", "private-cookie")
                    .set_body_string("private-body"),
            )
            .mount(&server)
            .await;
        let http = http(&server);
        let output = Writer::default();
        let url = format!("{}/private-path?token=private-secret", server.uri());
        http.execute(http.client.get(&url), RetryPolicy::Never)
            .with_subscriber(output.subscriber())
            .await
            .unwrap();
        // A malformed request fails without contacting any network service.
        let error = http
            .client
            .get("http://[private-secret")
            .send()
            .await
            .unwrap_err();
        tracing::subscriber::with_default(output.subscriber(), || network(&error, "send", 1));
        let text = output.text();
        assert!(
            text.contains("status=403") && text.contains("Upstream transport failed"),
            "{text}"
        );
        assert!(!text.contains("private-") && !text.contains(&server.uri()));
    }
}
