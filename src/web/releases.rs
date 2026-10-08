use std::time::Duration;

use reqwest::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, time::Instant};
use tokio_util::sync::CancellationToken;

const RELEASES: &str = "https://github.com/ohne-b/twitch-drops-miner/releases";
const MANIFEST: &str =
    "https://github.com/ohne-b/twitch-drops-miner/releases/latest/download/latest.json";
const MAX_MANIFEST: usize = 64 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Serialize)]
pub(super) struct ReleaseInfo {
    current_version: &'static str,
    latest_version: Option<String>,
    update_available: bool,
    check_succeeded: bool,
    download_url: String,
}
impl Default for ReleaseInfo {
    fn default() -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION"),
            latest_version: None,
            update_available: false,
            check_succeeded: false,
            download_url: RELEASES.into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    version: String,
}

pub(super) struct Releases {
    client: Client,
    cached: Mutex<Option<(Instant, ReleaseInfo)>>,
}
impl Releases {
    pub fn new() -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .user_agent("twitch-drops-miner")
                .timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::limited(3))
                .build()?,
            cached: Mutex::new(None),
        })
    }

    pub async fn check(&self, cancel: &CancellationToken) -> ReleaseInfo {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => ReleaseInfo::default(),
            info = self.check_url(MANIFEST) => info,
        }
    }

    async fn check_url(&self, url: &str) -> ReleaseInfo {
        // Coalesce dashboard requests and bound checks, including failed checks.
        let mut cached = self.cached.lock().await;
        if let Some((at, info)) = &*cached
            && at.elapsed() < CHECK_INTERVAL
        {
            return info.clone();
        }
        let info = self.fetch(url).await.unwrap_or_default();
        *cached = Some((Instant::now(), info.clone()));
        info
    }

    async fn fetch(&self, url: &str) -> Option<ReleaseInfo> {
        let mut response = self.client.get(url).send().await.ok()?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|size| size > MAX_MANIFEST as u64)
        {
            return None;
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if body.len() + chunk.len() > MAX_MANIFEST {
                return None;
            }
            body.extend_from_slice(&chunk);
        }
        let manifest: Manifest = serde_json::from_slice(&body).ok()?;
        let latest = Version::parse(&manifest.version).ok()?;
        if manifest.schema_version != 1 || manifest.version.len() > 128 || !latest.pre.is_empty() {
            return None;
        }
        let current = Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo version");
        Some(ReleaseInfo {
            latest_version: Some(latest.to_string()),
            update_available: latest.cmp_precedence(&current).is_gt(),
            check_succeeded: true,
            // Derive a trusted release link; never follow links supplied by the manifest.
            download_url: format!("{RELEASES}/tag/v{latest}"),
            ..ReleaseInfo::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    #[tokio::test]
    async fn release_checks_compare_precedence_and_only_link_to_this_repository() {
        let current = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        let stable = Version::new(current.major, current.minor, current.patch);
        let stable_is_newer = !current.pre.is_empty();
        for (version, available) in [
            ("99.0.0".to_owned(), true),
            (stable.to_string(), stable_is_newer),
            ("0.0.1".to_owned(), false),
            (format!("{stable}+build.99"), stable_is_newer),
        ] {
            let server = MockServer::start().await;
            Mock::given(path("/latest.json"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "schemaVersion": 1, "version": version,
                    "notes": if version == stable.to_string() {
                        "[Release notes](https://github.com/ohne-b/twitch-miner/releases/tag/v0.1.0)"
                    } else {
                        "<script>bad</script>"
                    },
                    "release_url":"https://evil.test",
                    "pub_date":"2026-10-08T12:00:00Z",
                    "platforms":{"windows-x86_64-nsis":{"url":"https://evil.test/installer.exe","signature":"not executable by the dashboard"}}
                })))
                .expect(1)
                .mount(&server)
                .await;
            let check = Releases::new().unwrap();
            let url = format!("{}/latest.json", server.uri());
            for _ in 0..2 {
                let info = check.check_url(&url).await;
                assert!(info.check_succeeded);
                assert_eq!(info.update_available, available);
                assert_eq!(info.download_url, format!("{RELEASES}/tag/v{version}"));
            }
        }
    }

    #[tokio::test]
    async fn concurrent_checks_share_a_result_and_expired_failures_are_retried() {
        let server = MockServer::start().await;
        Mock::given(path("/latest.json"))
            .respond_with(ResponseTemplate::new(503).set_delay(Duration::from_millis(20)))
            .expect(1)
            .mount(&server)
            .await;
        let check = Releases::new().unwrap();
        let url = format!("{}/latest.json", server.uri());
        let (first, second) = tokio::join!(check.check_url(&url), check.check_url(&url));
        assert!(!first.check_succeeded && !second.check_succeeded);
        server.verify().await;
        server.reset().await;
        Mock::given(path("/latest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "schemaVersion":1, "version":"99.0.0"
            })))
            .expect(1)
            .mount(&server)
            .await;
        tokio::time::pause();
        tokio::time::advance(CHECK_INTERVAL).await;
        tokio::time::resume();
        assert!(check.check_url(&url).await.update_available);
    }

    #[tokio::test]
    async fn malformed_missing_oversized_and_prerelease_manifests_are_not_up_to_date() {
        for response in [
            ResponseTemplate::new(404),
            ResponseTemplate::new(503),
            ResponseTemplate::new(200).set_body_string("bad json"),
            ResponseTemplate::new(200).set_body_string("x".repeat(MAX_MANIFEST + 1)),
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"schemaVersion":2,"version":"99.0.0"})),
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"schemaVersion":1,"version":"v99.0.0"})),
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"schemaVersion":1,"version":"99.0.0-rc.1"})),
        ] {
            let server = MockServer::start().await;
            Mock::given(path("/latest.json"))
                .respond_with(response)
                .mount(&server)
                .await;
            let info = Releases::new()
                .unwrap()
                .check_url(&format!("{}/latest.json", server.uri()))
                .await;
            assert!(!info.check_succeeded);
            assert!(!info.update_available);
            assert!(info.latest_version.is_none());
        }
        let check = Releases::new().unwrap();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(!check.check(&cancelled).await.check_succeeded);
    }
}
