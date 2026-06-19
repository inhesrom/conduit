//! Local control channel for `conduit web status` / `conduit web shutdown`.
//!
//! The running server binds a Unix socket at `~/.config/conduit/web.sock` and
//! answers one framed-JSON request per connection, reusing the daemon's
//! length-prefixed framing (`conduit_core::ipc`). It's local-only (socket file,
//! mode 0600), so it needs no auth — the same way session daemons are reached.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

use conduit_core::ipc::{read_frame, write_frame};

use crate::{config_dir, ServerState};

/// Where the running web server exposes its control socket.
pub fn control_socket_path() -> PathBuf {
    config_dir().join("web.sock")
}

#[derive(Serialize, Deserialize)]
pub enum ControlRequest {
    Status,
    Shutdown,
}

#[derive(Serialize, Deserialize)]
pub enum ControlResponse {
    Status(StatusReport),
    ShuttingDown,
}

#[derive(Serialize, Deserialize)]
pub struct StatusReport {
    pub pid: u32,
    pub url: String,
    pub tls: bool,
    pub auth_enabled: bool,
    pub uptime_secs: u64,
    pub clients: Vec<ClientReport>,
}

#[derive(Serialize, Deserialize)]
pub struct ClientReport {
    pub addr: String,
    pub session: String,
    pub connected_secs: u64,
}

// ---- server side -----------------------------------------------------------

/// Accept loop for the control socket. Runs until the process exits; a
/// `Shutdown` request fires `shutdown` so `serve()` can stop gracefully.
pub async fn serve_control(
    state: ServerState,
    url: String,
    tls: bool,
    shutdown: broadcast::Sender<()>,
) -> Result<()> {
    let path = control_socket_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Clear any stale socket from a previous (crashed) run before binding.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let state = state.clone();
        let url = url.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            let req = match read_frame(&mut stream).await {
                Ok(Some(bytes)) => match serde_json::from_slice::<ControlRequest>(&bytes) {
                    Ok(r) => r,
                    Err(_) => return,
                },
                _ => return,
            };
            let resp = match req {
                ControlRequest::Status => ControlResponse::Status(report(&state, &url, tls)),
                ControlRequest::Shutdown => {
                    let _ = shutdown.send(());
                    ControlResponse::ShuttingDown
                }
            };
            if let Ok(bytes) = serde_json::to_vec(&resp) {
                let _ = write_frame(&mut stream, &bytes).await;
            }
        });
    }
}

fn report(state: &ServerState, url: &str, tls: bool) -> StatusReport {
    let now = Instant::now();
    let clients = state
        .clients
        .lock()
        .map(|c| {
            c.values()
                .map(|ci| ClientReport {
                    addr: ci.addr.to_string(),
                    session: ci.session.clone(),
                    connected_secs: now.duration_since(ci.connected).as_secs(),
                })
                .collect()
        })
        .unwrap_or_default();
    StatusReport {
        pid: std::process::id(),
        url: url.to_string(),
        tls,
        auth_enabled: state.auth.enabled(),
        uptime_secs: now.duration_since(state.started).as_secs(),
        clients,
    }
}

// ---- client side (used by the `conduit web …` subcommands) -----------------

async fn request(req: ControlRequest) -> Result<ControlResponse> {
    let path = control_socket_path();
    let mut stream = UnixStream::connect(&path)
        .await
        .map_err(|_| anyhow!("web server not running"))?;
    let bytes = serde_json::to_vec(&req)?;
    write_frame(&mut stream, &bytes).await?;
    let frame = read_frame(&mut stream)
        .await?
        .ok_or_else(|| anyhow!("web server closed the connection"))?;
    Ok(serde_json::from_slice(&frame)?)
}

/// Fetch live status from the running web server.
pub async fn status() -> Result<StatusReport> {
    match request(ControlRequest::Status).await? {
        ControlResponse::Status(r) => Ok(r),
        ControlResponse::ShuttingDown => Err(anyhow!("unexpected response from web server")),
    }
}

/// Ask the running web server to stop gracefully.
pub async fn shutdown() -> Result<()> {
    match request(ControlRequest::Shutdown).await? {
        ControlResponse::ShuttingDown => Ok(()),
        ControlResponse::Status(_) => Err(anyhow!("unexpected response from web server")),
    }
}
