//! Session daemon: runs the core event loop behind a Unix socket so multiple
//! clients (TUI attach, web proxy) can drive the same live session. Both the
//! `conduit` binary's `--run-daemon` mode and any embedder call this.
//!
//! The daemon runs no web server — the standalone `conduit web serve` attaches
//! to this socket and bridges browser WebSockets to it.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use protocol::Command;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{mpsc, Mutex};

use crate::history::{snapshot_and_subscribe, spawn_recorder, CombinedHistory};
use crate::ipc::{read_frame, write_frame};
use crate::sessions::session_socket_path;
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
