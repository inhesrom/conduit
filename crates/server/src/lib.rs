//! Embedded web server: bridges browser WebSocket clients onto the same
//! Command/Event protocol the TUI speaks over the daemon's Unix socket.

pub mod assets;
pub mod fs;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use conduit_core::history::CombinedHistory;
use conduit_core::CoreHandle;
use tokio::sync::Mutex;

pub const DEFAULT_WEB_PORT: u16 = 3001;

#[derive(Clone)]
pub struct ServerState {
    pub core: CoreHandle,
    pub history: Arc<Mutex<CombinedHistory>>,
}

#[derive(Debug, Clone)]
pub struct WebConfig {
    pub bind: SocketAddr,
}

impl WebConfig {
    /// Resolve from `CONDUIT_WEB_PORT` / `CONDUIT_WEB_BIND` /
    /// `CONDUIT_DISABLE_EMBEDDED_WEB`. Returns `None` when the embedded web
    /// server is disabled.
    ///
    /// Defaults to localhost only. `CONDUIT_WEB_BIND` is an explicit opt-in
    /// for trusted private networks (e.g. a Tailscale tailnet) — there is no
    /// auth or TLS yet, so a reachable port means full terminal access.
    pub fn from_env() -> Option<Self> {
        if std::env::var_os("CONDUIT_DISABLE_EMBEDDED_WEB").is_some() {
            return None;
        }
        let port = std::env::var("CONDUIT_WEB_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(DEFAULT_WEB_PORT);
        let host = std::env::var("CONDUIT_WEB_BIND")
            .ok()
            .and_then(|v| v.parse::<std::net::IpAddr>().ok())
            .unwrap_or_else(|| std::net::IpAddr::from([127, 0, 0, 1]));
        if !host.is_loopback() {
            eprintln!(
                "[conduit] WARNING: web server binding {host} with NO auth/TLS — \
                 anyone who can reach it gets terminal access; only use on a \
                 trusted private network"
            );
        }
        Some(Self {
            bind: SocketAddr::new(host, port),
        })
    }
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<ServerState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws::handle_socket(socket, state))
}

async fn healthz() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "conduit",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn serve(
    core: CoreHandle,
    history: Arc<Mutex<CombinedHistory>>,
    cfg: WebConfig,
) -> anyhow::Result<()> {
    let state = ServerState { core, history };
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/fs/list", get(fs::list_dir))
        .route("/healthz", get(healthz))
        .fallback(assets::static_handler)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    eprintln!(
        "[conduit] embedded web server listening on http://{}",
        cfg.bind
    );
    axum::serve(listener, app).await?;
    Ok(())
}
