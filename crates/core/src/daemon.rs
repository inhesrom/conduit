//! Session daemon: runs the core event loop behind a Unix socket so multiple
//! clients (TUI attach, web proxy) can drive the same live session. Both the
//! `conduit` binary's hidden `run-daemon` subcommand and any embedder call this.
//!
//! The daemon runs no web server — `conduit web` attaches to this socket and
//! bridges browser WebSockets to it.

use std::path::PathBuf;
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
    load_registry, registered_session, save_registry, session_socket_path, socket_alive,
    validate_session_name, RegisteredSession, SessionEntry,
};
use crate::spawn_core;

/// Run a session daemon for `name`: spawn the core event loop, bind the
/// session's Unix socket, and bridge each client connection to the core
/// (replaying history on connect). Runs until the listener errors.
pub async fn run_session_daemon(name: &str) -> Result<()> {
    let sock_path = session_socket_path(name)?;
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Remove stale socket file if it exists
    let _ = std::fs::remove_file(&sock_path);

    let core = spawn_core();
    let listener = tokio::net::UnixListener::bind(&sock_path)
        .with_context(|| format!("failed to bind unix socket: {}", sock_path.display()))?;

    // Clean up socket on exit
    struct CleanupGuard(PathBuf);
    impl Drop for CleanupGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _guard = CleanupGuard(sock_path.clone());

    // Shared history buffer for replaying events to reconnecting clients
    let history = Arc::new(Mutex::new(CombinedHistory::new()));

    // Background task: record replayable events into history
    spawn_recorder(&core.evt_tx, history.clone());

    loop {
        let (stream, _) = listener.accept().await?;
        let (mut reader, mut writer) = stream.into_split();
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

/// Result of creating or reviving a session daemon.
#[derive(Debug, Clone)]
pub enum NewSessionOutcome {
    Created(SessionEntry),
    Revived(SessionEntry),
}

impl NewSessionOutcome {
    pub fn entry(&self) -> &SessionEntry {
        match self {
            Self::Created(entry) | Self::Revived(entry) => entry,
        }
    }
}

/// Result of attaching to a registered session daemon.
#[derive(Debug, Clone)]
pub enum AttachSessionOutcome {
    Attached(SessionEntry),
    Revived(SessionEntry),
}

impl AttachSessionOutcome {
    pub fn entry(&self) -> &SessionEntry {
        match self {
            Self::Attached(entry) | Self::Revived(entry) => entry,
        }
    }
}

/// Create a missing session daemon or revive a stale registered one.
///
/// A running session is a collision: callers should attach to it instead.
pub async fn new_session(name: &str) -> Result<NewSessionOutcome> {
    let state = registered_session(name)?;
    match plan_new_session(&state) {
        NewSessionAction::Create => Ok(NewSessionOutcome::Created(start_daemon(name).await?)),
        NewSessionAction::Revive => Ok(NewSessionOutcome::Revived(start_daemon(name).await?)),
        NewSessionAction::RejectRunning => Err(anyhow!(
            "session '{}' is already running; use `conduit attach {}`",
            name,
            name
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewSessionAction {
    Create,
    Revive,
    RejectRunning,
}

fn plan_new_session(state: &RegisteredSession) -> NewSessionAction {
    match state {
        RegisteredSession::Missing => NewSessionAction::Create,
        RegisteredSession::Stale(_) => NewSessionAction::Revive,
        RegisteredSession::Running(_) => NewSessionAction::RejectRunning,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachSessionAction {
    Attach,
    Revive,
    RejectMissing,
}

fn plan_attach_session(state: &RegisteredSession) -> AttachSessionAction {
    match state {
        RegisteredSession::Running(_) => AttachSessionAction::Attach,
        RegisteredSession::Stale(_) => AttachSessionAction::Revive,
        RegisteredSession::Missing => AttachSessionAction::RejectMissing,
    }
}

/// Attach to a registered session, reviving a stale daemon first.
///
/// Missing sessions are rejected so users choose creation explicitly with
/// `conduit new <name>`.
pub async fn attach_session(name: &str) -> Result<AttachSessionOutcome> {
    let state = registered_session(name)?;
    match plan_attach_session(&state) {
        AttachSessionAction::Attach => match state {
            RegisteredSession::Running(entry) => Ok(AttachSessionOutcome::Attached(entry)),
            _ => unreachable!("attach plan requires running state"),
        },
        AttachSessionAction::Revive => Ok(AttachSessionOutcome::Revived(start_daemon(name).await?)),
        AttachSessionAction::RejectMissing => Err(anyhow!(
            "session '{}' not found; create it with `conduit new {}`",
            name,
            name
        )),
    }
}

/// Spawn or respawn a detached daemon and update the registry only after its
/// socket is ready. If startup fails, the existing stale registry entry remains
/// intact.
async fn start_daemon(name: &str) -> Result<SessionEntry> {
    validate_session_name(name)?;
    let pid = spawn_daemon_process(name)?;
    let sock_path = session_socket_path(name)?;
    let sock_str = sock_path.display().to_string();

    wait_for_socket(&sock_str, Duration::from_secs(8)).await?;

    let entry = SessionEntry {
        name: name.to_string(),
        socket_path: sock_str,
        pid,
    };
    let mut registry = load_registry()?;
    registry.sessions.retain(|s| s.name != name);
    registry.sessions.push(entry.clone());
    save_registry(&registry)?;
    Ok(entry)
}

/// Spawn a detached `conduit run-daemon --session-name <name>` process and
/// return its pid. The child re-execs the current binary.
fn spawn_daemon_process(name: &str) -> Result<u32> {
    let exe = std::env::current_exe()?;
    let child = OsCommand::new(exe)
        .env("CONDUIT_SESSION_NAME", name)
        .arg("run-daemon")
        .arg("--session-name")
        .arg(name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> SessionEntry {
        SessionEntry {
            name: name.to_string(),
            socket_path: format!("/tmp/{name}.sock"),
            pid: 123,
        }
    }

    #[test]
    fn new_session_plan_handles_missing_running_and_stale() {
        assert_eq!(
            plan_new_session(&RegisteredSession::Missing),
            NewSessionAction::Create
        );
        assert_eq!(
            plan_new_session(&RegisteredSession::Stale(entry("work"))),
            NewSessionAction::Revive
        );
        assert_eq!(
            plan_new_session(&RegisteredSession::Running(entry("work"))),
            NewSessionAction::RejectRunning
        );
    }

    #[test]
    fn attach_session_plan_handles_missing_running_and_stale() {
        assert_eq!(
            plan_attach_session(&RegisteredSession::Missing),
            AttachSessionAction::RejectMissing
        );
        assert_eq!(
            plan_attach_session(&RegisteredSession::Running(entry("work"))),
            AttachSessionAction::Attach
        );
        assert_eq!(
            plan_attach_session(&RegisteredSession::Stale(entry("work"))),
            AttachSessionAction::Revive
        );
    }
}
