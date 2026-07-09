//! Standalone web server: serves the web UI, authenticates, and relays each
//! browser WebSocket to a chosen running session daemon's Unix socket — so the
//! browser can attach to and drive already-running sessions (and their live
//! agents), the way the TUI's `conduit tui attach` does.

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
    extract::{ConnectInfo, Path, Query, Request, State, WebSocketUpgrade},
    http::{
        header::{HOST, ORIGIN},
        HeaderMap, StatusCode, Uri,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;
use serde_json::json;

use auth::Auth;
use tls::TlsSource;

use conduit_core::history::CombinedHistory;
use conduit_core::CoreHandle;
use tokio::sync::Mutex as AsyncMutex;

pub const DEFAULT_WEB_PORT: u16 = 3001;
/// Fixed loopback port for the desktop window's in-process server. Kept stable
/// across launches (unlike an ephemeral port) so the webview's origin — and thus
/// its localStorage-backed settings — survives a restart. Distinct from
/// `DEFAULT_WEB_PORT` so it never collides with a running `conduit web`.
pub const DESKTOP_WEB_PORT: u16 = 3017;
const SESSION_COOKIE: &str = "conduit_session";

#[derive(Clone)]
pub struct ServerState {
    pub auth: Arc<Auth>,
    /// When set, every WS attaches to this one session and the chooser is hidden.
    pub pinned_session: Option<String>,
    /// Live WebSocket clients, keyed by a monotonic id — surfaced by `web status`.
    pub clients: Arc<Mutex<HashMap<u64, ClientInfo>>>,
    pub next_client_id: Arc<AtomicU64>,
    /// Process start time, for uptime reporting.
    pub started: Instant,
    /// When set, WebSockets bridge directly to this in-process core instead of
    /// proxying to a session daemon's Unix socket (used by the desktop app).
    pub embedded: Option<EmbeddedCore>,
    /// True when this server backs the native desktop window. Surfaced to the
    /// web UI (so it always shows the session chooser on startup) and gates the
    /// local-only session-creation endpoint.
    pub desktop: bool,
}

/// An in-process core that the server bridges browser WebSockets straight to,
/// replaying history on connect — the same contract the daemon socket provides,
/// but without a separate process or Unix socket. See `serve_embedded`.
#[derive(Clone)]
pub struct EmbeddedCore {
    pub core: CoreHandle,
    pub history: Arc<AsyncMutex<CombinedHistory>>,
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
    let clients = state.clients.clone();
    let id = state.next_client_id.fetch_add(1, Ordering::Relaxed);

    // Embedded mode: bridge straight to the in-process core — no daemon socket.
    if let Some(embedded) = state.embedded.clone() {
        return ws.on_upgrade(move |socket| async move {
            clients.lock().unwrap().insert(
                id,
                ClientInfo {
                    addr,
                    session: name,
                    connected: Instant::now(),
                },
            );
            let _guard = ClientGuard {
                clients: clients.clone(),
                id,
            };
            proxy::handle_embedded(socket, embedded).await;
        });
    }

    let path = match conduit_core::sessions::session_socket_path(&name) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, "bad session name").into_response(),
    };
    ws.on_upgrade(move |socket| async move {
        clients.lock().unwrap().insert(
            id,
            ClientInfo {
                addr,
                session: name,
                connected: Instant::now(),
            },
        );
        // Deregisters on every exit path of the relay (close, error, daemon EOF).
        let _guard = ClientGuard {
            clients: clients.clone(),
            id,
        };
        proxy::handle_proxy(socket, path).await;
    })
}

async fn sessions(State(state): State<ServerState>) -> impl IntoResponse {
    use conduit_core::sessions::SessionStatus;
    let list: Vec<SessionStatus> = match &state.pinned_session {
        // Pinned/embedded: one logical session bridged to an in-process core,
        // which has no daemon socket — report it as always running.
        Some(p) => vec![SessionStatus {
            name: p.clone(),
            running: true,
        }],
        // Full registry, including stale entries, so the chooser can show and
        // resurrect sessions whose daemon died (e.g. after a reboot).
        None => conduit_core::sessions::list_all_sessions(),
    };
    Json(json!({
        "sessions": list,
        "pinned": state.pinned_session.is_some(),
        "desktop": state.desktop,
    }))
}

#[derive(Deserialize)]
struct CreateSessionReq {
    name: String,
}

/// Start a session daemon and return its name. The local desktop window may
/// create missing sessions or revive stale ones. The shared `conduit web`
/// server may only attach/revive a registered session; minting brand-new names
/// there would spawn arbitrary processes on the host.
async fn create_session(
    State(state): State<ServerState>,
    Json(body): Json<CreateSessionReq>,
) -> Response {
    let name = body.name.trim();
    if let Err(e) = conduit_core::sessions::validate_session_name(name) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    if !state.desktop {
        match conduit_core::daemon::attach_session(name).await {
            Ok(outcome) => {
                return Json(json!({ "ok": true, "name": outcome.entry().name })).into_response()
            }
            Err(e) => {
                let status = match conduit_core::sessions::registered_session(name) {
                    Ok(conduit_core::sessions::RegisteredSession::Missing) => StatusCode::FORBIDDEN,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                return (status, Json(json!({ "ok": false, "error": e.to_string() })))
                    .into_response();
            }
        }
    }
    match conduit_core::daemon::new_session(name).await {
        Ok(outcome) => Json(json!({ "ok": true, "name": outcome.entry().name })).into_response(),
        Err(e) => {
            let status = match conduit_core::sessions::registered_session(name) {
                Ok(conduit_core::sessions::RegisteredSession::Running(_)) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, Json(json!({ "ok": false, "error": e.to_string() }))).into_response()
        }
    }
}

/// Delete a session entirely: stop its daemon, drop it from the registry, and
/// remove its persisted state. Desktop-only — the shared `conduit web` server
/// must not let a browser destroy host sessions (mirrors `create_session`).
/// The pinned/embedded session has no daemon and backs the live surface, so it
/// is rejected too.
async fn remove_session(State(state): State<ServerState>, Path(name): Path<String>) -> Response {
    if !state.desktop {
        return (StatusCode::FORBIDDEN, "deleting sessions is desktop-only").into_response();
    }
    if state.pinned_session.as_deref() == Some(name.as_str()) {
        return (StatusCode::BAD_REQUEST, "cannot delete the active session").into_response();
    }
    match conduit_core::sessions::remove_session(&name) {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )
            .into_response(),
    }
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
///
/// The effective host is the `Host` header on HTTP/1, but on HTTP/2 there is no
/// `Host` header — the authority arrives in the `:authority` pseudo-header,
/// which hyper surfaces via the request URI. We must check both, or every
/// browser login over the default HTTPS server (which negotiates HTTP/2) is
/// rejected as a bad origin. With no resolvable host we allow it, since the
/// `SameSite=Strict` session cookie is the primary CSRF protection.
fn origin_ok(headers: &HeaderMap, uri: &Uri) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true; // non-browser client (e.g. curl) — SameSite still guards
    };
    let Some(origin_host) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
    else {
        return false; // opaque/"null" or malformed Origin
    };
    let effective_host = headers
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| uri.authority().map(|a| a.as_str()));
    match effective_host {
        Some(host) => origin_host == host,
        None => true,
    }
}

async fn login(
    State(state): State<ServerState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    uri: Uri,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<LoginReq>,
) -> Response {
    if !origin_ok(&headers, &uri) {
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

/// Assemble the full router: public routes, auth-protected routes, and the
/// embedded static web UI with SPA fallback. Shared by `serve`/`serve_embedded`.
fn build_router(state: ServerState) -> Router {
    let protected = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/sessions", get(sessions).post(create_session))
        .route("/api/sessions/{name}", delete(remove_session))
        .route("/api/fs/list", get(fs::list_dir))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/session", get(session))
        .route("/healthz", get(healthz))
        .merge(protected)
        .fallback(assets::static_handler)
        .with_state(state)
}

/// Serve the web UI bound to an in-process core — no daemon, no TLS, no auth,
/// no control socket. Binds `bind` (pass port 0 for an ephemeral loopback
/// port), reports the actual bound address through `ready`, then serves until
/// Ctrl-C or the process exits. Pins every WebSocket to one session bridged to
/// the given in-process core. (The desktop app uses `serve_desktop` instead.)
pub async fn serve_embedded(
    bind: SocketAddr,
    pinned_session: String,
    embedded: EmbeddedCore,
    ready: tokio::sync::oneshot::Sender<SocketAddr>,
) -> anyhow::Result<()> {
    // Auth disabled: a loopback-only local server needs no password. Point the
    // auth/session paths at a guaranteed-absent location so nothing loads.
    let void = PathBuf::from("/nonexistent/conduit-desktop");
    let auth = Arc::new(Auth::load(void.clone(), void, false));
    let state = ServerState {
        auth,
        pinned_session: Some(pinned_session),
        clients: Arc::new(Mutex::new(HashMap::new())),
        next_client_id: Arc::new(AtomicU64::new(1)),
        started: Instant::now(),
        embedded: Some(embedded),
        desktop: false,
    };
    let app = build_router(state).into_make_service_with_connect_info::<SocketAddr>();

    let listener = tokio::net::TcpListener::bind(bind).await?;
    let local = listener.local_addr()?;
    let _ = ready.send(local);
    eprintln!("[conduit] embedded web server listening on http://{local}");

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

/// Serve the web UI for the native desktop window — no TLS, no auth, no control
/// socket — proxying WebSockets to the user's running session daemons. With
/// `pinned_session = None` the UI shows the session chooser on startup; with
/// `Some(name)` it locks the window to that one session. Binds `bind` (pass port
/// 0 for an ephemeral loopback port), reports the actual bound address through
/// `ready`, then serves until Ctrl-C or the process exits.
pub async fn serve_desktop(
    bind: SocketAddr,
    pinned_session: Option<String>,
    ready: tokio::sync::oneshot::Sender<SocketAddr>,
) -> anyhow::Result<()> {
    // Auth disabled: a loopback-only local server needs no password. Point the
    // auth/session paths at a guaranteed-absent location so nothing loads.
    let void = PathBuf::from("/nonexistent/conduit-desktop");
    let auth = Arc::new(Auth::load(void.clone(), void, false));
    let state = ServerState {
        auth,
        pinned_session,
        clients: Arc::new(Mutex::new(HashMap::new())),
        next_client_id: Arc::new(AtomicU64::new(1)),
        started: Instant::now(),
        embedded: None,
        desktop: true,
    };
    let app = build_router(state).into_make_service_with_connect_info::<SocketAddr>();

    // Prefer the fixed desktop port (stable origin → persistent settings). If
    // it's taken (e.g. a second desktop window), fall back to an ephemeral port
    // so this window still runs — it just won't share the persisted settings.
    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(l) => l,
        Err(_) if bind.port() != 0 => {
            eprintln!(
                "[conduit] desktop port {} busy; using an ephemeral port",
                bind.port()
            );
            tokio::net::TcpListener::bind((bind.ip(), 0)).await?
        }
        Err(e) => return Err(e.into()),
    };
    let local = listener.local_addr()?;
    let _ = ready.send(local);
    eprintln!("[conduit] desktop web server listening on http://{local}");

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

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
        embedded: None,
        desktop: false,
    };

    let app = build_router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderName;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    // HTTP/1: the authority is in the `Host` header; the URI is path-only.
    #[test]
    fn origin_ok_http1_uses_host_header() {
        let uri: Uri = "/api/login".parse().unwrap();
        assert!(origin_ok(
            &headers(&[
                ("host", "127.0.0.1:3001"),
                ("origin", "https://127.0.0.1:3001")
            ]),
            &uri
        ));
        assert!(!origin_ok(
            &headers(&[
                ("host", "127.0.0.1:3001"),
                ("origin", "https://evil.example")
            ]),
            &uri
        ));
    }

    // HTTP/2: no `Host` header — the authority rides in the URI. This is the
    // case every browser hits over the default HTTPS server.
    #[test]
    fn origin_ok_http2_uses_uri_authority() {
        let uri: Uri = "https://127.0.0.1:3001/api/login".parse().unwrap();
        assert!(origin_ok(
            &headers(&[("origin", "https://127.0.0.1:3001")]),
            &uri
        ));
        assert!(!origin_ok(
            &headers(&[("origin", "https://evil.example")]),
            &uri
        ));
    }

    // Non-browser clients (curl) send no Origin; SameSite=Strict still guards.
    #[test]
    fn origin_ok_no_origin_allowed() {
        let uri: Uri = "/api/login".parse().unwrap();
        assert!(origin_ok(&headers(&[]), &uri));
        assert!(origin_ok(&headers(&[("host", "127.0.0.1:3001")]), &uri));
    }

    // An opaque ("null") or schemeless Origin is rejected.
    #[test]
    fn origin_ok_opaque_origin_rejected() {
        let uri: Uri = "https://127.0.0.1:3001/api/login".parse().unwrap();
        assert!(!origin_ok(&headers(&[("origin", "null")]), &uri));
    }
}
