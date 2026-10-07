mod releases;
pub mod socket;
#[cfg(test)]
mod tests;

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::{NaiveDate, NaiveDateTime};
use serde::Deserialize;
use serde_json::{Value, json};
use socketioxide::SocketIo;
use tokio::sync::mpsc;

use crate::{
    auth::{
        AuthAction, AuthError, AuthSettingsRequest, LoginRequest, WebAuth, session_cookie,
        token_from_headers, unix_now,
    },
    dto::SettingsView,
    origin::DashboardOrigin,
    store::HistoryFilter,
};
use socket::SocketHub;

use crate::app::{AppError, Application, ENGLISH};
pub use crate::app::{Command, CommandRequest, message};

pub struct WebState {
    pub application: Arc<Application>,
    pub auth: Arc<WebAuth>,
    pub origin: DashboardOrigin,
    pub sockets: SocketHub,
    releases: releases::Releases,
    #[cfg(feature = "dashboard-fixture")]
    pub fixture: bool,
}

// Routes compose the application with transport security; the miner owns no HTTP state.
impl std::ops::Deref for WebState {
    type Target = Arc<Application>;
    fn deref(&self) -> &Self::Target {
        &self.application
    }
}

pub type App = WebState;

impl WebState {
    pub fn open(
        directory: PathBuf,
        public_base_url: &str,
    ) -> Result<(Arc<Self>, mpsc::Receiver<CommandRequest>)> {
        let (application, receiver) = Application::open(directory)?;
        let auth = Arc::new(WebAuth::open(&application.data.path)?);
        Ok((
            Arc::new(Self {
                application,
                sockets: SocketHub::new(auth.clone()),
                auth,
                origin: DashboardOrigin::new(public_base_url)?,
                releases: releases::Releases::new()?,
                #[cfg(feature = "dashboard-fixture")]
                fixture: false,
            }),
            receiver,
        ))
    }
}

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub &'static str);

async fn games(
    State(app): State<Arc<App>>,
    Json(query): Json<crate::app::commands::GameQuery>,
) -> Result<Json<Vec<crate::config::GameMetadata>>, ApiError> {
    if !query.valid() {
        return Err(ApiError::invalid());
    }
    let sender = app
        .game_queries
        .read()
        .await
        .clone()
        .ok_or_else(ApiError::unavailable)?;
    let (complete, result) = tokio::sync::oneshot::channel();
    sender
        .try_send(crate::app::commands::GameRequest { query, complete })
        .map_err(|_| ApiError::unavailable())?;
    let games = tokio::time::timeout(Duration::from_secs(12), result)
        .await
        .map_err(|_| ApiError::unavailable())?
        .map_err(|_| ApiError::unavailable())?
        .map_err(|_| ApiError::unavailable())?;
    Ok(Json(games))
}
impl ApiError {
    fn invalid() -> Self {
        Self(StatusCode::BAD_REQUEST, "invalid_request")
    }
    fn unavailable() -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, "request_failed")
    }
}
impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::ShuttingDown => Self(StatusCode::CONFLICT, "shutting_down"),
            AppError::LoginRequired => Self(StatusCode::CONFLICT, "twitch_login_required"),
            AppError::SettingsConflict => Self(StatusCode::CONFLICT, "settings_conflict"),
            AppError::InvalidSettings => Self(StatusCode::BAD_REQUEST, "invalid_settings"),
            AppError::Unavailable => Self::unavailable(),
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"detail":self.1}))).into_response()
    }
}
impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let mut response =
            (self.status(), Json(json!({"detail":self.to_string()}))).into_response();
        if self == Self::RateLimited {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "60".parse().unwrap());
        }
        response
    }
}

pub fn router(app: Arc<App>) -> Router {
    let (layer, io) = SocketIo::builder().max_payload(1024 * 1024).build_layer();
    SocketHub::attach(&app, io);
    let mut routes = Router::new()
        .route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/", get(index))
        .route("/campaigns", get(index))
        .route("/history", get(index))
        .route("/activity", get(index))
        .route("/settings", get(index))
        .route("/login", get(login_page))
        .route("/assets/{*path}", get(asset))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/auth/settings", post(auth_settings))
        .route("/api/status", get(status))
        .route("/api/channels", get(channels))
        .route("/api/channels/select", post(select_channel))
        .route("/api/campaigns", get(campaigns))
        .route("/api/console", get(console))
        .route("/api/settings", get(settings).post(update_settings))
        .route("/api/games", post(games))
        .route("/api/settings/verify-proxy", post(verify_proxy))
        .route("/api/version", get(version))
        .route("/api/history", get(history))
        .route("/api/history/stats", get(history_stats))
        .route("/api/twitch/logout", post(logout_twitch))
        .route("/api/oauth/confirm", post(confirm_oauth))
        .route("/api/reload", post(reload))
        .route("/api/cache/clear", post(clear_cache))
        .route("/api/mode/exit-manual", post(exit_manual))
        .route("/api/close", post(close));
    #[cfg(feature = "dashboard-fixture")]
    if app.fixture {
        routes = super::fixture::routes(routes);
    }
    routes = routes
        .layer(layer)
        .layer(middleware::from_fn_with_state(app.clone(), guard));
    routes.with_state(app)
}

async fn guard(State(app): State<Arc<App>>, mut request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let secure_transport = request.uri().scheme_str() == Some("https");
    if !app
        .origin
        .permits(request.method(), &path, request.headers(), secure_transport)
    {
        return private(ApiError(StatusCode::FORBIDDEN, "forbidden").into_response());
    }
    let asset =
        path.starts_with("/assets/") && matches!(*request.method(), Method::GET | Method::HEAD);
    let public = asset
        || matches!(
            path.as_str(),
            "/login" | "/healthz" | "/api/auth/status" | "/api/auth/login"
        );
    #[cfg(feature = "dashboard-fixture")]
    let public = public
        || app.fixture
            && matches!(
                path.as_str(),
                "/__test/health" | "/__test/reset" | "/__test/event"
            );
    if !public
        && !app
            .auth
            .state
            .lock()
            .await
            .allowed(&token_from_headers(request.headers()), unix_now())
    {
        let response = if matches!(
            path.as_str(),
            "/" | "/campaigns" | "/history" | "/activity" | "/settings"
        ) {
            Redirect::to("/login").into_response()
        } else {
            ApiError(StatusCode::UNAUTHORIZED, "authentication_required").into_response()
        };
        return private(response);
    }
    // Bound every submitted document and suppress extractor errors that could echo secrets.
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) && !path.starts_with("/socket.io")
    {
        let limit = if path.starts_with("/api/auth/") {
            16384
        } else {
            1024 * 1024
        };
        let (parts, body) = request.into_parts();
        let Ok(body) = to_bytes(body, limit).await else {
            return private(
                ApiError(StatusCode::PAYLOAD_TOO_LARGE, "invalid_request").into_response(),
            );
        };
        request = Request::from_parts(parts, Body::from(body));
    }
    let response = next.run(request).await;
    if asset && response.status().is_success() {
        response
    } else {
        private(response)
    }
}

fn private(mut response: Response) -> Response {
    if !response.headers().contains_key(header::CACHE_CONTROL) {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    }
    response
        .headers_mut()
        .insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());
    response
        .headers_mut()
        .insert(header::REFERRER_POLICY, "same-origin".parse().unwrap());
    response
}

#[derive(rust_embed::RustEmbed)]
#[folder = "web/"]
struct Assets;

async fn index() -> Response {
    serve_asset("index.html", false)
}
async fn login_page(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if app
        .auth
        .state
        .lock()
        .await
        .allowed(&token_from_headers(&headers), unix_now())
    {
        Redirect::to("/").into_response()
    } else {
        index().await
    }
}
async fn asset(Path(path): Path<String>) -> Response {
    serve_asset(&format!("assets/{path}"), true)
}
fn serve_asset(path: &str, immutable: bool) -> Response {
    if path
        .split('/')
        .any(|part| matches!(part, "." | "..") || part.contains('\\'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(asset) = Assets::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (
                header::CONTENT_TYPE,
                mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .to_string(),
            ),
            (
                header::CACHE_CONTROL,
                if immutable {
                    "public, max-age=31536000, immutable"
                } else {
                    "no-cache"
                }
                .to_owned(),
            ),
        ],
        asset.data.into_owned(),
    )
        .into_response()
}

async fn document<T: serde::de::DeserializeOwned>(request: Request) -> Result<T, ApiError> {
    let bytes = to_bytes(request.into_body(), 1024 * 1024)
        .await
        .map_err(|_| ApiError::invalid())?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::invalid())
}

async fn auth_status(State(app): State<Arc<App>>, headers: HeaderMap) -> Json<Value> {
    let state = app.auth.state.lock().await;
    Json(
        json!({"enabled":state.enabled(),"authenticated":state.allowed(&token_from_headers(&headers),unix_now()),"translations":ENGLISH["gui"]["auth"]}),
    )
}

fn peer(request: &Request) -> Option<std::net::IpAddr> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|v| v.0.ip())
}
fn cookie_secure(app: &App, request: &Request) -> bool {
    app.origin
        .secure_cookie(request.uri().scheme_str() == Some("https"))
}
async fn auth_response(
    app: &App,
    token: &str,
    remember: bool,
    secure: bool,
    disconnect_all: bool,
) -> Response {
    app.sockets.prune(disconnect_all).await;
    let enabled = app.auth.state.lock().await.enabled();
    (
        [(header::SET_COOKIE, session_cookie(token, remember, secure))],
        Json(json!({"success":true,"enabled":enabled})),
    )
        .into_response()
}
async fn auth_login(State(app): State<Arc<App>>, request: Request) -> Response {
    let previous = token_from_headers(request.headers());
    let peer = peer(&request);
    let secure = cookie_secure(&app, &request);
    let data = match document::<LoginRequest>(request).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let remember = data.remember;
    match app.auth.login(data, &previous, peer).await {
        Ok(token) => auth_response(&app, &token, remember, secure, false).await,
        Err(e) => e.into_response(),
    }
}
async fn auth_logout(State(app): State<Arc<App>>, request: Request) -> Response {
    let token = token_from_headers(request.headers());
    let secure = cookie_secure(&app, &request);
    match app.auth.logout(&token).await {
        Ok(()) => auth_response(&app, "", false, secure, false).await,
        Err(e) => e.into_response(),
    }
}
async fn auth_settings(State(app): State<Arc<App>>, request: Request) -> Response {
    let token = token_from_headers(request.headers());
    let peer = peer(&request);
    let secure = cookie_secure(&app, &request);
    let data = match document::<AuthSettingsRequest>(request).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let disable = data.action == AuthAction::Disable;
    match app.auth.configure(data, &token, peer).await {
        Ok(token) => auth_response(&app, &token, false, secure, disable).await,
        Err(e) => e.into_response(),
    }
}

async fn status(State(app): State<Arc<App>>) -> Json<Value> {
    let state = app.snapshot.read().await;
    Json(json!({"status":state.status,"login":state.login,"manual_mode":state.manual_mode}))
}
async fn channels(State(app): State<Arc<App>>) -> Json<Value> {
    Json(json!({"channels":app.snapshot.read().await.channels}))
}
async fn campaigns(State(app): State<Arc<App>>) -> Json<Value> {
    Json(json!({"campaigns":app.snapshot.read().await.campaigns}))
}
async fn console(State(app): State<Arc<App>>) -> Json<Value> {
    Json(json!({"lines":app.snapshot.read().await.console}))
}
async fn settings(State(app): State<Arc<App>>) -> Json<SettingsView> {
    Json(app.snapshot.read().await.settings.clone())
}

async fn update_settings(
    State(app): State<Arc<App>>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let patch: Value = document(request).await?;
    let permit = app
        .settings_slot
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::unavailable())?;
    let owned = app.clone();
    // Once a write starts, an HTTP disconnect cannot leave disk and live state
    // disagreeing. Shutdown drains the transaction before releasing the data lock.
    app.writes
        .spawn(async move {
            let result = save_settings(owned.clone(), patch).await;
            // The miner can append a manually selected game during reconfiguration.
            // Never hold the settings transaction while waiting for it to drain.
            drop(permit);
            if result.is_ok() {
                owned.command(Command::SettingsChanged).await?;
            }
            result
        })
        .await
        .map_err(|_| ApiError::unavailable())?
}

async fn save_settings(app: Arc<App>, patch: Value) -> Result<Json<Value>, ApiError> {
    let settings = app
        .change_settings(|current| {
            if patch
                .get("revision")
                .is_some_and(|v| !v.is_null() && v.as_str() != Some(current.revision.as_str()))
            {
                return Err(AppError::SettingsConflict);
            }
            current
                .values
                .patched(&patch)
                .map_err(|_| AppError::InvalidSettings)
        })
        .await?;
    Ok(Json(json!({"success":true,"settings":settings})))
}

async fn select_channel(
    State(app): State<Arc<App>>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Selection {
        channel_id: Option<u64>,
        channel: Option<String>,
        duration_minutes: Option<u32>,
    }
    let selection: Selection = document(request).await?;
    if app.snapshot.read().await.login.user_id.is_none() {
        return Err(ApiError(StatusCode::CONFLICT, "twitch_login_required"));
    }
    if selection
        .duration_minutes
        .is_some_and(|minutes| !(1..=1440).contains(&minutes))
    {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid_manual_duration"));
    }
    let duration = selection
        .duration_minutes
        .map(|minutes| Duration::from_secs(u64::from(minutes) * 60));
    if selection.channel.is_some() == selection.channel_id.is_some() {
        return Err(ApiError::invalid());
    }
    if let Some(channel) = selection.channel {
        let login = crate::twitch::channels::channel_login(&channel)
            .ok_or(ApiError(StatusCode::BAD_REQUEST, "invalid_channel"))?;
        app.command(Command::MineChannel(login, duration)).await?;
        return Ok(Json(json!({"success":true})));
    }
    let channel_id = selection.channel_id.unwrap();
    if !app
        .snapshot
        .read()
        .await
        .channels
        .iter()
        .any(|c| c.id == channel_id)
    {
        return Err(ApiError(StatusCode::NOT_FOUND, "channel_not_found"));
    }
    app.command(Command::SelectChannel(channel_id, duration))
        .await?;
    Ok(Json(json!({"success":true})))
}

#[derive(Deserialize, Default)]
struct HistoryQuery {
    game: Option<String>,
    campaign_id: Option<String>,
    since: Option<String>,
    limit: Option<usize>,
}
impl HistoryQuery {
    fn filter(self) -> HistoryFilter {
        let since = self.since.and_then(|s| {
            s.parse()
                .ok()
                .or_else(|| {
                    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                        .ok()
                        .and_then(|d| d.and_hms_opt(0, 0, 0))
                        .map(|d| d.and_utc())
                })
                .or_else(|| {
                    NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S")
                        .ok()
                        .map(|d| d.and_utc())
                })
        });
        HistoryFilter {
            game: self.game.filter(|s| !s.is_empty()),
            campaign_id: self.campaign_id,
            since,
            limit: self.limit.filter(|n| *n > 0).map(|n| n.min(5000)),
        }
    }
}
async fn history(State(app): State<Arc<App>>, Query(query): Query<HistoryQuery>) -> Json<Value> {
    let history = app.history.lock().await;
    let state = app.snapshot.read().await;
    Json(
        json!({"total":history.total(),"entries":history.entries(&query.filter()),"instance":state.instance,"revision":state.history_revision,"clear_revision":state.history_clear_revision}),
    )
}
async fn history_stats(State(app): State<Arc<App>>) -> Json<Value> {
    Json(app.history.lock().await.stats())
}
async fn run_command(app: &App, command: Command) -> Result<Json<Value>, ApiError> {
    app.command(command).await?;
    Ok(Json(json!({"success":true})))
}
async fn reload(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    app.refresh_inventory().await?;
    Ok(Json(json!({"success":true})))
}
async fn clear_cache(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    app.clear_cache().await?;
    Ok(Json(json!({"success":true})))
}
async fn logout_twitch(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    run_command(&app, Command::Logout).await
}
async fn confirm_oauth(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    run_command(&app, Command::ConfirmOAuth).await
}
async fn exit_manual(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    run_command(&app, Command::ExitManual).await
}
async fn close(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    run_command(&app, Command::Shutdown).await
}

async fn verify_proxy(
    State(app): State<Arc<App>>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    #[derive(Deserialize)]
    struct Proxy {
        proxy: String,
    }
    let Proxy { proxy } = document(request).await?;
    crate::config::validate_proxy(&proxy).map_err(|_| ApiError::invalid())?;
    if proxy.is_empty() {
        return Ok(Json(
            json!({"success":false,"message":message("gui.backend.proxy_empty",&[])}),
        ));
    }
    #[cfg(feature = "dashboard-fixture")]
    if app.fixture {
        return Ok(Json(json!({"success":true})));
    }
    let _ = app;
    let started = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(&proxy).map_err(|_| ApiError::invalid())?)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| ApiError::unavailable())?;
    let response = client.get("https://www.twitch.tv").send().await;
    Ok(Json(match response {
        Ok(response) if response.status().as_u16() < 500 => {
            json!({"success":true,"latency":started.elapsed().as_millis(),"message":message("gui.backend.proxy_connected",&[])})
        }
        _ => json!({"success":false,"message":message("gui.backend.proxy_failed",&[])}),
    }))
}

async fn version(State(app): State<Arc<App>>) -> Json<releases::ReleaseInfo> {
    #[cfg(feature = "dashboard-fixture")]
    if app.fixture {
        return Json(releases::ReleaseInfo::default());
    }
    Json(app.releases.check(&app.shutdown).await)
}
