use std::{
    collections::{BTreeMap, VecDeque},
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::{
    sync::{Mutex, Semaphore},
    time::Instant,
};

use crate::store::{atomic_json, read_json};

pub const COOKIE_NAME: &str = "tdm_session";
pub const SESSION_SECONDS: u32 = 30 * 24 * 60 * 60;
const MAX_SESSIONS: usize = 128;

pub fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

use twitch_drops_miner_core::random_hex;

pub fn token_from_headers(headers: &HeaderMap) -> String {
    for header in headers.get_all(http::header::COOKIE) {
        if let Ok(header) = header.to_str() {
            for cookie in cookie::Cookie::split_parse(header).flatten() {
                if cookie.name() == COOKIE_NAME && cookie.value().len() <= 256 {
                    return cookie.value().to_owned();
                }
            }
        }
    }
    String::new()
}

pub fn session_cookie(token: &str, remember: bool, secure: bool) -> String {
    let mut cookie = cookie::Cookie::build((COOKIE_NAME, token.to_owned()))
        .path("/")
        .http_only(true)
        .same_site(cookie::SameSite::Strict)
        .secure(secure);
    if token.is_empty() {
        cookie = cookie.max_age(cookie::time::Duration::ZERO);
    } else if remember {
        cookie = cookie.max_age(cookie::time::Duration::seconds(i64::from(SESSION_SECONDS)));
    }
    cookie.build().to_string()
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("invalid_password")]
    InvalidPassword,
    #[error("password_length")]
    PasswordLength,
    #[error("password_mismatch")]
    PasswordMismatch,
    #[error("authentication_required")]
    Required,
    #[error("auth_changed")]
    Changed,
    #[error("rate_limited")]
    RateLimited,
    #[error("request_failed")]
    Storage,
}

impl AuthError {
    pub fn status(&self) -> http::StatusCode {
        use http::StatusCode as S;
        match self {
            Self::InvalidPassword | Self::Required => S::UNAUTHORIZED,
            Self::PasswordLength | Self::PasswordMismatch => S::BAD_REQUEST,
            Self::Changed => S::CONFLICT,
            Self::RateLimited => S::TOO_MANY_REQUESTS,
            Self::Storage => S::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuthAction {
    Enable,
    Change,
    Disable,
}

#[derive(Deserialize)]
pub struct AuthSettingsRequest {
    pub action: AuthAction,
    #[serde(default)]
    pub current_password: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub confirm_password: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct AuthFile {
    version: u32,
    password_hash: String,
    sessions: BTreeMap<String, f64>,
}

impl Default for AuthFile {
    fn default() -> Self {
        Self {
            version: 1,
            password_hash: String::new(),
            sessions: BTreeMap::new(),
        }
    }
}

pub struct AuthState {
    file: AuthFile,
    attempts: VecDeque<(Instant, Option<IpAddr>)>,
}

impl AuthState {
    pub fn enabled(&self) -> bool {
        !self.file.password_hash.is_empty()
    }
    pub fn digest(token: &str) -> String {
        hex::encode(Sha256::digest(token.as_bytes()))
    }
    pub fn expires_at(&self, token: &str) -> Option<f64> {
        (!token.is_empty())
            .then(|| self.file.sessions.get(&Self::digest(token)).copied())
            .flatten()
    }
    pub fn authenticated(&self, token: &str, now: f64) -> bool {
        self.expires_at(token).is_some_and(|expiry| expiry > now)
    }
    pub fn allowed(&self, token: &str, now: f64) -> bool {
        !self.enabled() || self.authenticated(token, now)
    }

    fn limit(&mut self, peer: Option<IpAddr>, now: Instant) -> Result<(), AuthError> {
        while self
            .attempts
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) >= Duration::from_secs(60))
        {
            self.attempts.pop_front();
        }
        if self.attempts.len() >= 30
            || self.attempts.iter().filter(|(_, ip)| *ip == peer).count() >= 5
        {
            return Err(AuthError::RateLimited);
        }
        self.attempts.push_back((now, peer));
        Ok(())
    }
}

pub struct WebAuth {
    path: PathBuf,
    pub state: Mutex<AuthState>,
    hash_slots: Arc<Semaphore>,
}

impl WebAuth {
    pub fn open(directory: &Path) -> Result<Self> {
        let path = directory.join("web_auth.json");
        let file = read_json::<AuthFile>(&path)
            .context("invalid dashboard authentication file")?
            .unwrap_or_default();
        let hex_length = |value: &str, length| {
            value.len() == length
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        let valid_hash = || {
            let parts: Vec<_> = file.password_hash.split('$').collect();
            parts.len() == 3
                && parts[0] == "scrypt"
                && hex_length(parts[1], 32)
                && hex_length(parts[2], 64)
        };
        if file.version != 1
            || !file.password_hash.is_empty() && !valid_hash()
            || file.sessions.len() > MAX_SESSIONS
            || file.password_hash.is_empty() && !file.sessions.is_empty()
            || file
                .sessions
                .iter()
                .any(|(key, expiry)| !hex_length(key, 64) || !expiry.is_finite() || *expiry <= 0.0)
        {
            bail!("invalid dashboard authentication file");
        }
        Ok(Self {
            path,
            state: Mutex::new(AuthState {
                file,
                attempts: VecDeque::new(),
            }),
            hash_slots: Arc::new(Semaphore::new(1)),
        })
    }

    fn save(&self, state: &mut AuthState, file: AuthFile) -> Result<(), AuthError> {
        atomic_json(&self.path, &file).map_err(|_| AuthError::Storage)?;
        state.file = file;
        Ok(())
    }

    async fn hash(&self, password: String, salt: String) -> Result<String, AuthError> {
        let permit = self
            .hash_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AuthError::Storage)?;
        tokio::task::spawn_blocking(move || {
            // The permit lives in the blocking job even if the HTTP request is cancelled.
            let _permit = permit;
            let salt_bytes = hex::decode(&salt).map_err(|_| AuthError::Storage)?;
            let params = scrypt::Params::new(15, 8, 3).map_err(|_| AuthError::Storage)?;
            let mut key = [0; 32];
            scrypt::scrypt(password.as_bytes(), &salt_bytes, &params, &mut key)
                .map_err(|_| AuthError::Storage)?;
            Ok(format!("scrypt${salt}${}", hex::encode(key)))
        })
        .await
        .map_err(|_| AuthError::Storage)?
    }

    async fn verify(&self, password: String, hash: &str) -> Result<(), AuthError> {
        if !(1..=1024).contains(&password.chars().count()) {
            return Err(AuthError::InvalidPassword);
        }
        let salt = hash.split('$').nth(1).ok_or(AuthError::Storage)?.to_owned();
        let candidate = self.hash(password, salt).await?;
        if !bool::from(candidate.as_bytes().ct_eq(hash.as_bytes())) {
            return Err(AuthError::InvalidPassword);
        }
        Ok(())
    }

    fn new_token() -> Result<String, AuthError> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| AuthError::Storage)?;
        Ok(URL_SAFE_NO_PAD.encode(bytes))
    }

    pub async fn login(
        &self,
        request: LoginRequest,
        previous: &str,
        peer: Option<IpAddr>,
    ) -> Result<String, AuthError> {
        let mut state = self.state.lock().await;
        state.limit(peer, Instant::now())?;
        if !state.enabled() {
            return Err(AuthError::Changed);
        }
        self.verify(request.password, &state.file.password_hash)
            .await?;
        let mut next = state.file.clone();
        let now = unix_now();
        next.sessions.retain(|_, expiry| *expiry > now);
        next.sessions.remove(&AuthState::digest(previous));
        while next.sessions.len() >= MAX_SESSIONS {
            let earliest = next
                .sessions
                .iter()
                .min_by(|a, b| a.1.total_cmp(b.1))
                .map(|(key, _)| key.clone())
                .expect("full session map");
            next.sessions.remove(&earliest);
        }
        let token = Self::new_token()?;
        next.sessions
            .insert(AuthState::digest(&token), now + f64::from(SESSION_SECONDS));
        self.save(&mut state, next)?;
        Ok(token)
    }

    pub async fn logout(&self, token: &str) -> Result<(), AuthError> {
        let mut state = self.state.lock().await;
        if !state.allowed(token, unix_now()) {
            return Err(AuthError::Required);
        }
        let mut next = state.file.clone();
        next.sessions.remove(&AuthState::digest(token));
        self.save(&mut state, next)
    }

    pub async fn configure(
        &self,
        request: AuthSettingsRequest,
        current: &str,
        peer: Option<IpAddr>,
    ) -> Result<String, AuthError> {
        let mut state = self.state.lock().await;
        state.limit(peer, Instant::now())?;
        if (request.action == AuthAction::Enable) == state.enabled() {
            return Err(AuthError::Changed);
        }
        if !state.allowed(current, unix_now()) {
            return Err(AuthError::Required);
        }
        if state.enabled() {
            self.verify(request.current_password, &state.file.password_hash)
                .await?;
        }
        if request.action == AuthAction::Disable {
            self.save(&mut state, AuthFile::default())?;
            return Ok(String::new());
        }
        if !(8..=1024).contains(&request.password.chars().count()) {
            return Err(AuthError::PasswordLength);
        }
        if request.password != request.confirm_password {
            return Err(AuthError::PasswordMismatch);
        }
        let salt = random_hex::<16>().map_err(|_| AuthError::Storage)?;
        let password_hash = self.hash(request.password, salt).await?;
        let token = Self::new_token()?;
        self.save(
            &mut state,
            AuthFile {
                version: 1,
                password_hash,
                sessions: BTreeMap::from([(
                    AuthState::digest(&token),
                    unix_now() + f64::from(SESSION_SECONDS),
                )]),
            },
        )?;
        Ok(token)
    }

    #[cfg(any(test, feature = "dashboard-fixture"))]
    pub async fn reset(&self) -> Result<(), AuthError> {
        let mut state = self.state.lock().await;
        self.save(&mut state, AuthFile::default())?;
        state.attempts.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PASSWORD: &str = "test password only";

    #[tokio::test]
    async fn existing_password_hash_format_matches_independent_scrypt_vector() {
        // Generated independently with Node crypto.scryptSync, N=32768,r=8,p=3.
        let dir = tempfile::tempdir().unwrap();
        let auth = WebAuth::open(dir.path()).unwrap();
        let expected = "scrypt$00112233445566778899aabbccddeeff$99dc8512ad81b88fedd32a73f10ce4f9d8ee1ff4ba02ff6f7311c35ca24aade6";
        assert_eq!(
            auth.hash(PASSWORD.into(), "00112233445566778899aabbccddeeff".into())
                .await
                .unwrap(),
            expected
        );
    }
    fn configure(action: AuthAction, current: &str) -> AuthSettingsRequest {
        AuthSettingsRequest {
            action,
            current_password: current.into(),
            password: PASSWORD.into(),
            confirm_password: PASSWORD.into(),
        }
    }
    fn login() -> LoginRequest {
        LoginRequest {
            password: PASSWORD.into(),
            remember: false,
        }
    }

    #[test]
    fn corrupt_protection_fails_closed_and_remains_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        for content in [
            "{",
            "{}",
            "{\"version\":1,\"password_hash\":\"broken\",\"sessions\":{}}",
            "{\"version\":1,\"password_hash\":\"\",\"sessions\":{\"bad\":42}}",
        ] {
            std::fs::write(dir.path().join("web_auth.json"), content).unwrap();
            assert!(WebAuth::open(dir.path()).is_err());
            assert_eq!(
                std::fs::read_to_string(dir.path().join("web_auth.json")).unwrap(),
                content
            );
        }
    }

    #[test]
    fn cookie_lifecycle_keeps_security_attributes() {
        for secure in [true, false] {
            for remember in [true, false] {
                let cookie = session_cookie("abc", remember, secure);
                assert!(
                    cookie.contains("HttpOnly")
                        && cookie.contains("SameSite=Strict")
                        && cookie.contains("Path=/")
                );
                assert_eq!(cookie.contains("Secure"), secure);
                assert_eq!(cookie.contains("Max-Age=2592000"), remember);
            }
            let deleted = session_cookie("", false, secure);
            assert!(deleted.contains("Max-Age=0"));
            assert_eq!(deleted.contains("Secure"), secure);
        }
        let headers = HeaderMap::from_iter([(
            http::header::COOKIE,
            "other=abc; tdm_session=valid".parse().unwrap(),
        )]);
        assert_eq!(token_from_headers(&headers), "valid");
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limits_bound_per_peer_and_global_work() {
        let mut state = AuthState {
            file: AuthFile::default(),
            attempts: VecDeque::new(),
        };
        let peer = Some("127.0.0.1".parse().unwrap());
        for _ in 0..5 {
            state.limit(peer, Instant::now()).unwrap();
        }
        assert_eq!(
            state.limit(peer, Instant::now()),
            Err(AuthError::RateLimited)
        );
        tokio::time::advance(Duration::from_secs(60)).await;
        for i in 0..30 {
            state
                .limit(Some(IpAddr::from([10, 0, 0, i])), Instant::now())
                .unwrap();
        }
        assert_eq!(
            state.limit(None, Instant::now()),
            Err(AuthError::RateLimited)
        );
        assert_eq!(state.attempts.len(), 30);
    }

    #[tokio::test]
    async fn session_rotation_revocation_restart_and_disable() {
        let dir = tempfile::tempdir().unwrap();
        let auth = WebAuth::open(dir.path()).unwrap();
        let first = auth
            .configure(configure(AuthAction::Enable, ""), "", None)
            .await
            .unwrap();
        let saved = std::fs::read_to_string(dir.path().join("web_auth.json")).unwrap();
        assert!(!saved.contains(&first) && !saved.contains(PASSWORD));
        assert!(
            WebAuth::open(dir.path())
                .unwrap()
                .state
                .lock()
                .await
                .authenticated(&first, unix_now())
        );
        let second = auth.login(login(), &first, None).await.unwrap();
        assert!(!auth.state.lock().await.authenticated(&first, unix_now()));
        let third = auth.login(login(), "", None).await.unwrap();
        auth.logout(&second).await.unwrap();
        assert!(!auth.state.lock().await.authenticated(&second, unix_now()));
        assert!(auth.state.lock().await.authenticated(&third, unix_now()));
        let changed = auth
            .configure(configure(AuthAction::Change, PASSWORD), &third, None)
            .await
            .unwrap();
        assert!(!auth.state.lock().await.authenticated(&third, unix_now()));
        let peer = Some("127.0.0.2".parse().unwrap());
        auth.configure(configure(AuthAction::Disable, PASSWORD), &changed, peer)
            .await
            .unwrap();
        assert!(!auth.state.lock().await.enabled());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(dir.path().join("web_auth.json")).unwrap()
            )
            .unwrap(),
            json!({"version":1,"password_hash":"","sessions":{}})
        );
    }

    #[tokio::test]
    async fn failure_does_not_commit_authentication_or_password_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut auth = WebAuth::open(dir.path()).unwrap();
        let token = auth
            .configure(configure(AuthAction::Enable, ""), "", None)
            .await
            .unwrap();
        let mut wrong = configure(AuthAction::Change, "wrong");
        assert_eq!(
            auth.configure(wrong, &token, None).await,
            Err(AuthError::InvalidPassword)
        );
        wrong = configure(AuthAction::Change, PASSWORD);
        wrong.confirm_password = "different".into();
        assert_eq!(
            auth.configure(wrong, &token, None).await,
            Err(AuthError::PasswordMismatch)
        );
        auth.path = dir.path().join("missing").join("auth.json");
        assert_eq!(auth.logout(&token).await, Err(AuthError::Storage));
        assert!(auth.state.lock().await.authenticated(&token, unix_now()));
    }

    #[test]
    fn session_expiry_uses_fixed_lifetime_with_exclusive_boundary() {
        let state = AuthState {
            file: AuthFile {
                password_hash: "protected".into(),
                sessions: BTreeMap::from([(AuthState::digest("token"), 100.0)]),
                version: 1,
            },
            attempts: VecDeque::new(),
        };
        assert!(state.allowed("token", 99.0));
        assert!(!state.allowed("token", 100.0));
        assert!(!state.allowed("", 99.0));
    }
}
