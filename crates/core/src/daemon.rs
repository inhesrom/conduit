//! Session daemon: runs the core event loop behind a Unix socket so multiple
//! clients (TUI attach, web proxy) can drive the same live session. Both the
//! `conduit` binary's hidden `run-daemon` subcommand and any embedder call this.
//!
//! The daemon runs no web server — `conduit web` attaches to this socket and
//! bridges browser WebSockets to it.

use std::process::{Command as OsCommand, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use protocol::Command;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{mpsc, Mutex};

use crate::history::{snapshot_and_subscribe, spawn_recorder, CombinedHistory};
use crate::ipc::{read_frame, write_frame};
use crate::sessions::{
    load_registry, save_registry, session_socket_path, socket_alive, SessionEntry,
};
use crate::spawn_core;
use crate::transport;

/// Run a session daemon for `name`: spawn the core event loop, bind the
/// session's Unix socket, and bridge each client connection to the core
/// (replaying history on connect). Runs until the listener errors.
pub async fn run_session_daemon(name: &str) -> Result<()> {
    // Platform endpoint: a Unix socket path or a Windows named pipe. The
    // transport handles binding, cleanup, and (on Unix) the parent dir + stale
    // socket removal that this used to do by hand.
    let endpoint = session_socket_path(name)?.to_string_lossy().into_owned();

    let core = spawn_core();
    let mut listener = transport::bind(&endpoint)
        .await
        .with_context(|| format!("failed to bind session endpoint: {endpoint}"))?;

    // Shared history buffer for replaying events to reconnecting clients
    let history = Arc::new(Mutex::new(CombinedHistory::new()));

    // Background task: record replayable events into history
    spawn_recorder(&core.evt_tx, history.clone());

    loop {
        let conn = listener.accept().await?;
        let (mut reader, mut writer) = tokio::io::split(conn);
        let cmd_tx = core.cmd_tx.clone();
        let history = history.clone();
        let core_evt_tx = core.evt_tx.clone();

        // Bridge: read Commands from socket, send Events back
        tokio::spawn(async move {
            // Write events to socket
            let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(2048);
            tokio::spawn(async move {
                while let Some(data) = write_rx.recv().await {
                    if write_frame(&mut writer, &data).await.is_err() {
                        break;
                    }
                }
            });

            // Snapshot history and subscribe to the broadcast atomically, then
            // send the replay after the lock is released so a slow client
            // can't stall the recorder.
            let (payloads, mut evt_rx) = snapshot_and_subscribe(&history, &core_evt_tx).await;
            for frame in payloads {
                if write_tx.send(frame).await.is_err() {
                    return;
                }
            }

            // Forward live broadcast events directly to socket writer
            let write_tx2 = write_tx.clone();
            tokio::spawn(async move {
                loop {
                    match evt_rx.recv().await {
                        Ok(evt) => {
                            if let Ok(payload) = serde_json::to_vec(&evt) {
                                if write_tx2.send(payload).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(RecvError::Closed) => break,
                        Err(RecvError::Lagged(n)) => {
                            eprintln!("[conduit] client event forwarder lagged by {n} events");
                            continue;
                        }
                    }
                }
            });

            // Read commands from socket
            loop {
                match read_frame(&mut reader).await {
                    Ok(Some(data)) => {
                        if let Ok(cmd) = serde_json::from_slice::<Command>(&data) {
                            if cmd_tx.send(cmd).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        });
    }
}

/// Ensure a session daemon named `name` is running, spawning a detached daemon
/// process if it isn't, and return its registry entry. Shared by the TUI's
/// session commands and the desktop app's "new session" flow.
pub async fn ensure_session_running(name: &str) -> Result<SessionEntry> {
    let mut registry = load_registry()?;
    if let Some(existing) = registry.sessions.iter().find(|s| s.name == name).cloned() {
        if socket_alive(&existing.socket_path) {
            return Ok(existing);
        }
        registry.sessions.retain(|s| s.name != name);
    }

    let pid = spawn_daemon_process(name)?;
    let sock_path = session_socket_path(name)?;
    let sock_str = sock_path.display().to_string();

    wait_for_socket(&sock_str, Duration::from_secs(8)).await?;

    let entry = SessionEntry {
        name: name.to_string(),
        socket_path: sock_str,
        pid,
    };
    registry.sessions.retain(|s| s.name != name);
    registry.sessions.push(entry.clone());
    save_registry(&registry)?;
    Ok(entry)
}

/// Spawn a detached `conduit run-daemon --session-name <name>` process and
/// return its pid. The child re-execs the current binary.
fn spawn_daemon_process(name: &str) -> Result<u32> {
    let exe = std::env::current_exe()?;
    let mut cmd = OsCommand::new(exe);
    cmd.env("CONDUIT_SESSION_NAME", name)
        .arg("run-daemon")
        .arg("--session-name")
        .arg(name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Fully detach on Windows so launching a session from the desktop app does
    // not flash a console window or tie the daemon's lifetime to the GUI process.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS gives the daemon no console at all (it already writes
        // to null); it's mutually exclusive with CREATE_NO_WINDOW, so don't
        // combine them. CREATE_NEW_PROCESS_GROUP keeps a parent console's Ctrl+C
        // from reaching the daemon.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn daemon for session '{}'", name))?;
    Ok(child.id())
}

/// Poll until the session's Unix socket accepts connections, or time out.
async fn wait_for_socket(path: &str, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if socket_alive(path) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    Err(anyhow!("daemon did not become ready at {}", path))
}
