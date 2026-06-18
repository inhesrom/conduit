//! Embedded web server: bridges browser WebSocket clients onto the same
//! Command/Event protocol the TUI speaks, serves the web UI, and (for remote
//! access) gates everything behind a password + TLS.

pub mod assets;
pub mod auth;
pub mod fs;
pub mod tls;
pub mod ws;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Request, State, WebSocketUpgrade},
    http::{
        header::{HOST, ORIGIN},
        HeaderMap, StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use conduit_core::history::CombinedHistory;
use conduit_core::CoreHandle;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;

use auth::Auth;
use tls::TlsSource;

pub const DEFAULT_WEB_PORT: u16 = 3001;
const SESSION_COOKIE: &str = "conduit_session";

#[derive(Clone)]
pub struct ServerState {
    pub core: CoreHandle,
    pub history: Arc<Mutex<CombinedHistory>>,
    pub auth: Arc<Auth>,
}

#[derive(Debug, Clone)]
pub struct WebConfig {
    pub bind: SocketAddr,
    pub tls: Option<TlsSource>,
    pub auth_path: PathBuf,
    pub sessions_path: PathBuf,
}

fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("conduit")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/conduit")
    } else {
        PathBuf::from(".")
    }
}

/// Path of the web password file (used by `conduit web set-password`).
pub fn web_auth_path() -> PathBuf {
    config_dir().join("web_auth.json")
}

impl WebConfig {
    /// Resolve from env + config dir. Returns `None` when the embedded web
    /// server is disabled or its configuration is refused.
    ///
    /// Policy: a non-loopback bind requires both a password and TLS — there is
    /// no safe way to expose terminal access without them.
    pub fn from_env() -> Option<Self> {
        if std::env::var_os("CONDUIT_DISABLE_EMBEDDED_WEB").is_some() {
            return None;
        }
        let dir = config_dir();
        let port = std::env::var("CONDUIT_WEB_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(DEFAULT_WEB_PORT);
        let host = std::env::var("CONDUIT_WEB_BIND")
            .ok()
            .and_then(|v| v.parse::<IpAddr>().ok())
            .unwrap_or_else(|| IpAddr::from([127, 0, 0, 1]));

        let auth_path = dir.join("web_auth.json");
        let has_password = auth_path.exists();

        // TLS: explicit cert/key, else self-signed when TLS is wanted.
        let cert = std::env::var("CONDUIT_WEB_CERT")
            .ok()
            .filter(|s| !s.is_empty());
        let key = std::env::var("CONDUIT_WEB_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        let want_tls = !host.is_loopback()
            || matches!(
                std::env::var("CONDUIT_WEB_TLS").as_deref(),
                Ok("on" | "1" | "auto")
            );
        let tls = match (cert, key) {
            (Some(c), Some(k)) => Some(TlsSource::Files {
                cert: PathBuf::from(c),
                key: PathBuf::from(k),
            }),
            _ if want_tls => Some(TlsSource::SelfSigned {
                dir: dir.join("web_tls"),
            }),
            _ => None,
        };

        if !host.is_loopback() && (!has_password || tls.is_none()) {
            eprintln!(
                "[conduit] refusing to bind web server to {host}: a non-localhost bind \
                 requires a password (`conduit web set-password`) and TLS. Staying off."
            );
            return None;
        }

        Some(Self {
            bind: SocketAddr::new(host, port),
            tls,
            auth_path,
            sessions_path: dir.join("web_sessions.json"),
        })
    }
}

// ---- handlers --------------------------------------------------------------

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<ServerState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws::handle_socket(socket, state))
}

async fn healthz() -> impl IntoResponse {
    Json(json!({ "name": "conduit", "version": env!("CARGO_PKG_VERSION") }))
}

async fn session(State(state): State<ServerState>, jar: CookieJar) -> impl IntoResponse {
    let token = jar.get(SESSION_COOKIE).map(|c| c.value().to_string());
    Json(json!({
        "auth_required": state.auth.enabled(),
        "authenticated": state.auth.validate(token.as_deref()),
    }))
}

#[derive(Deserialize)]
struct LoginReq {
    password: String,
}

/// Reject cross-origin form posts (defense in depth atop SameSite cookies).
fn origin_ok(headers: &HeaderMap) -> bool {
    let origin = headers.get(ORIGIN).and_then(|v| v.to_str().ok());
    let host = headers.get(HOST).and_then(|v| v.to_str().ok());
    match origin {
        None => true,
        Some(o) => {
            let stripped = o
                .strip_prefix("https://")
                .or_else(|| o.strip_prefix("http://"));
            matches!((stripped, host), (Some(oh), Some(h)) if oh == h)
        }
    }
}

async fn login(
    State(state): State<ServerState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<LoginReq>,
) -> Response {
    if !origin_ok(&headers) {
        return (StatusCode::FORBIDDEN, "bad origin").into_response();
    }
    if !state.auth.enabled() {
        return Json(json!({ "ok": true })).into_response();
    }
    if state.auth.rate_limited(addr.ip()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts; wait a few minutes",
        )
            .into_response();
    }
    match state.auth.login(addr.ip(), &body.password) {
        Some(token) => {
            let mut cookie = Cookie::new(SESSION_COOKIE, token);
            cookie.set_http_only(true);
            cookie.set_same_site(SameSite::Strict);
            cookie.set_path("/");
            cookie.set_secure(state.auth.secure_cookie);
            (jar.add(cookie), Json(json!({ "ok": true }))).into_response()
        }
        None => (StatusCode::UNAUTHORIZED, Json(json!({ "ok": false }))).into_response(),
    }
}

async fn logout(State(state): State<ServerState>, jar: CookieJar) -> Response {
    let token = jar.get(SESSION_COOKIE).map(|c| c.value().to_string());
    state.auth.logout(token.as_deref());
    (
        jar.remove(Cookie::from(SESSION_COOKIE)),
        Json(json!({ "ok": true })),
    )
        .into_response()
}

async fn require_auth(
    State(state): State<ServerState>,
    jar: CookieJar,
    req: Request,
    next: Next,
) -> Response {
    let token = jar.get(SESSION_COOKIE).map(|c| c.value().to_string());
    if state.auth.validate(token.as_deref()) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "authentication required").into_response()
    }
}

// ---- serve -----------------------------------------------------------------

pub async fn serve(
    core: CoreHandle,
    history: Arc<Mutex<CombinedHistory>>,
    cfg: WebConfig,
) -> anyhow::Result<()> {
    let auth = Arc::new(Auth::load(
        cfg.auth_path.clone(),
        cfg.sessions_path.clone(),
        cfg.tls.is_some(),
    ));
    if auth.enabled() {
        eprintln!("[conduit] web auth enabled (password required)");
    }
    let state = ServerState {
        core,
        history,
        auth,
    };

    let protected = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/fs/list", get(fs::list_dir))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let app = Router::new()
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/session", get(session))
        .route("/healthz", get(healthz))
        .merge(protected)
        .fallback(assets::static_handler)
        .with_state(state)
        .into_make_service_with_connect_info::<SocketAddr>();

    let scheme = if cfg.tls.is_some() { "https" } else { "http" };
    eprintln!(
        "[conduit] embedded web server listening on {scheme}://{}",
        cfg.bind
    );

    match &cfg.tls {
        Some(src) => {
            let tls_config = tls::rustls_config(src).await?;
            axum_server::bind_rustls(cfg.bind, tls_config)
                .serve(app)
                .await?;
        }
        None => {
            let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
            axum::serve(listener, app).await?;
        }
    }
    Ok(())
}
