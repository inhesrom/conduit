//! Standalone web server: serves the web UI, authenticates, and relays each
//! browser WebSocket to a chosen running session daemon's Unix socket — so the
//! browser can attach to and drive already-running sessions (and their live
//! agents), the way the TUI's `conduit -a` does.

pub mod assets;
pub mod auth;
pub mod control;
pub mod fs;
pub mod proxy;
pub mod tls;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::{
    extract::{ConnectInfo, Query, Request, State, WebSocketUpgrade},
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
use serde::Deserialize;
use serde_json::json;

use auth::Auth;
use tls::TlsSource;

pub const DEFAULT_WEB_PORT: u16 = 3001;
const SESSION_COOKIE: &str = "conduit_session";

#[derive(Clone)]
pub struct ServerState {
    pub auth: Arc<Auth>,
    /// When set, every WS attaches to this one session and the picker is hidden.
    pub pinned_session: Option<String>,
    /// Live WebSocket clients, keyed by a monotonic id — surfaced by `web status`.
    pub clients: Arc<Mutex<HashMap<u64, ClientInfo>>>,
    pub next_client_id: Arc<AtomicU64>,
    /// Process start time, for uptime reporting.
    pub started: Instant,
}

/// One connected browser WebSocket, tracked for `conduit web status`.
#[derive(Clone)]
pub struct ClientInfo {
    pub addr: SocketAddr,
    pub session: String,
    pub connected: Instant,
}

/// Removes a client from `ServerState::clients` when its connection ends,
/// covering every exit path of the proxy relay via `Drop`.
struct ClientGuard {
    clients: Arc<Mutex<HashMap<u64, ClientInfo>>>,
    id: u64,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.remove(&self.id);
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebConfig {
    pub bind: SocketAddr,
    pub tls: Option<TlsSource>,
    pub auth_path: PathBuf,
    pub sessions_path: PathBuf,
    pub pinned_session: Option<String>,
}

pub(crate) fn config_dir() -> PathBuf {
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
    /// Resolve from env, pinning to `session` if given. Returns `None` (with a
    /// logged reason) when a non-loopback bind lacks the required password+TLS.
    pub fn from_env(pinned_session: Option<String>) -> Option<Self> {
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

        let cert = std::env::var("CONDUIT_WEB_CERT")
            .ok()
            .filter(|s| !s.is_empty());
        let key = std::env::var("CONDUIT_WEB_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        // HTTPS is on by default (self-signed cert for localhost, generated on
        // first run). Opt out with CONDUIT_WEB_TLS=off; a non-localhost bind
        // always requires TLS regardless of the env var.
        let want_tls = !host.is_loopback()
            || !matches!(
                std::env::var("CONDUIT_WEB_TLS").as_deref(),
                Ok("off" | "0" | "false")
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
                 requires a password (`conduit web set-password`) and TLS."
            );
            return None;
        }

        Some(Self {
            bind: SocketAddr::new(host, port),
            tls,
            auth_path,
            sessions_path: dir.join("web_sessions.json"),
            pinned_session,
        })
    }
}

// ---- handlers --------------------------------------------------------------

#[derive(Deserialize)]
struct WsQuery {
    session: Option<String>,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
    Query(q): Query<WsQuery>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    let Some(name) = state.pinned_session.clone().or(q.session) else {
        return (StatusCode::BAD_REQUEST, "missing ?session").into_response();
    };
    let path = match conduit_core::sessions::session_socket_path(&name) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, "bad session name").into_response(),
    };
    let clients = state.clients.clone();
    let id = state.next_client_id.fetch_add(1, Ordering::Relaxed);
    ws.on_upgrade(move |socket| async move {
        clients.lock().unwrap().insert(
            id,
            ClientInfo { addr, session: name, connected: Instant::now() },
        );
        // Deregisters on every exit path of the relay (close, error, daemon EOF).
        let _guard = ClientGuard { clients: clients.clone(), id };
        proxy::handle_proxy(socket, path).await;
    })
}

async fn sessions(State(state): State<ServerState>) -> impl IntoResponse {
    let names: Vec<String> = match &state.pinned_session {
        Some(p) => vec![p.clone()],
        None => conduit_core::sessions::list_running_sessions()
            .into_iter()
            .map(|s| s.name)
            .collect(),
    };
    Json(json!({ "sessions": names, "pinned": state.pinned_session.is_some() }))
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

pub async fn serve(cfg: WebConfig) -> anyhow::Result<()> {
    let auth = Arc::new(Auth::load(
        cfg.auth_path.clone(),
        cfg.sessions_path.clone(),
        cfg.tls.is_some(),
    ));
    if auth.enabled() {
        eprintln!("[conduit] web auth enabled (password required)");
    }
    let state = ServerState {
        auth,
        pinned_session: cfg.pinned_session.clone(),
        clients: Arc::new(Mutex::new(HashMap::new())),
        next_client_id: Arc::new(AtomicU64::new(1)),
        started: Instant::now(),
    };

    let protected = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/sessions", get(sessions))
        .route("/api/fs/list", get(fs::list_dir))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let app = Router::new()
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/session", get(session))
        .route("/healthz", get(healthz))
        .merge(protected)
        .fallback(assets::static_handler)
        .with_state(state.clone())
        .into_make_service_with_connect_info::<SocketAddr>();

    let scheme = if cfg.tls.is_some() { "https" } else { "http" };
    let url = format!("{scheme}://{}", cfg.bind);
    eprintln!("[conduit] web server listening on {url}");

    // Local control channel for `conduit web status` / `conduit web shutdown`.
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    {
        let state = state.clone();
        let url = url.clone();
        let tls = cfg.tls.is_some();
        let shutdown_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = control::serve_control(state, url, tls, shutdown_tx).await {
                eprintln!("[conduit] control socket unavailable: {e}");
            }
        });
    }

    // Resolves on a control `Shutdown` request, SIGTERM, or Ctrl-C.
    let mut shutdown_rx = shutdown_tx.subscribe();
    let shutdown = async move {
        let ctrl_c = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        #[cfg(unix)]
        let term = async {
            use tokio::signal::unix::{signal, SignalKind};
            if let Ok(mut s) = signal(SignalKind::terminate()) {
                s.recv().await;
            }
        };
        #[cfg(not(unix))]
        let term = std::future::pending::<()>();
        tokio::select! {
            _ = shutdown_rx.recv() => {}
            _ = ctrl_c => {}
            _ = term => {}
        }
    };

    match &cfg.tls {
        Some(src) => {
            let tls_config = tls::rustls_config(src).await?;
            let handle = axum_server::Handle::new();
            tokio::spawn({
                let handle = handle.clone();
                async move {
                    shutdown.await;
                    handle.graceful_shutdown(Some(std::time::Duration::from_secs(2)));
                }
            });
            axum_server::bind_rustls(cfg.bind, tls_config)
                .handle(handle)
                .serve(app)
                .await?;
        }
        None => {
            let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await?;
        }
    }

    // Best-effort cleanup so a later `web status` doesn't find a stale socket.
    let _ = std::fs::remove_file(control::control_socket_path());
    Ok(())
}
