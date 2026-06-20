//! Session proxy: relays a browser WebSocket to a running session daemon's
//! Unix socket. Both speak the same JSON protocol, so this is a pure reframe —
//! WS Text ⇄ length-prefixed frame, no (de)serialization. The daemon replays
//! its history on connect, so the browser gets the session's full live state
//! (including already-running agents) the moment it attaches.

use std::path::PathBuf;

use axum::extract::ws::{Message, WebSocket};
use conduit_core::history::snapshot_and_subscribe;
use conduit_core::ipc::{read_frame, write_frame};
use protocol::Command;
use tokio::net::UnixStream;
use tokio::sync::broadcast::error::RecvError;

use crate::EmbeddedCore;

pub async fn handle_proxy(mut socket: WebSocket, socket_path: PathBuf) {
    let stream = match UnixStream::connect(&socket_path).await {
        Ok(s) => s,
        Err(e) => {
            let err =
                serde_json::json!({ "Error": { "message": format!("session unavailable: {e}") } });
            let _ = socket.send(Message::Text(err.to_string().into())).await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let (mut reader, mut writer) = stream.into_split();

    loop {
        tokio::select! {
            // daemon -> browser
            frame = read_frame(&mut reader) => {
                match frame {
                    Ok(Some(bytes)) => match String::from_utf8(bytes) {
                        Ok(text) => {
                            if socket.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => continue,
                    },
                    Ok(None) | Err(_) => break, // daemon closed or framing error
                }
            }
            // browser -> daemon
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        if write_frame(&mut writer, txt.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}

/// Bridge a browser WebSocket directly to an in-process core (no daemon socket).
/// Mirrors the daemon's per-connection contract: replay the history snapshot on
/// connect, then forward live events, while relaying browser commands into the
/// core. Used by the desktop app's embedded server.
pub async fn handle_embedded(mut socket: WebSocket, embedded: EmbeddedCore) {
    let EmbeddedCore { core, history } = embedded;

    // Snapshot history and subscribe atomically, then send the replay so a
    // late-connecting client sees the session's full live state.
    let (payloads, mut evt_rx) = snapshot_and_subscribe(&history, &core.evt_tx).await;
    for frame in payloads {
        match String::from_utf8(frame) {
            Ok(text) => {
                if socket.send(Message::Text(text.into())).await.is_err() {
                    return;
                }
            }
            Err(_) => continue,
        }
    }

    loop {
        tokio::select! {
            // core -> browser
            evt = evt_rx.recv() => {
                match evt {
                    Ok(evt) => {
                        let Ok(payload) = serde_json::to_vec(&evt) else { continue };
                        match String::from_utf8(payload) {
                            Ok(text) => {
                                if socket.send(Message::Text(text.into())).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => continue,
                        }
                    }
                    Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(n)) => {
                        eprintln!("[conduit] embedded event forwarder lagged by {n} events");
                        continue;
                    }
                }
            }
            // browser -> core
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        if let Ok(cmd) = serde_json::from_slice::<Command>(txt.as_bytes()) {
                            if core.cmd_tx.send(cmd).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}
