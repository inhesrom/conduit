//! WebSocket bridge: Command JSON in, Event JSON out, with history replay
//! on connect — the WS twin of the daemon's Unix-socket client handler.

use axum::extract::ws::{Message, WebSocket};
use conduit_core::history::snapshot_and_subscribe;
use protocol::Command;
use tokio::sync::broadcast::error::RecvError;

use crate::ServerState;

pub async fn handle_socket(mut socket: WebSocket, state: ServerState) {
    let (payloads, mut evt_rx) = snapshot_and_subscribe(&state.history, &state.core.evt_tx).await;
    for payload in payloads {
        let Ok(text) = String::from_utf8(payload) else {
            continue;
        };
        if socket.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            maybe_msg = socket.recv() => {
                match maybe_msg {
                    Some(Ok(Message::Text(txt))) => {
                        if let Ok(cmd) = serde_json::from_str::<Command>(txt.as_str()) {
                            if state.core.cmd_tx.send(cmd).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
            evt = evt_rx.recv() => {
                match evt {
                    Ok(evt) => {
                        let Ok(text) = serde_json::to_string(&evt) else {
                            continue;
                        };
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(n)) => {
                        // A lagged stream has dropped terminal bytes and would
                        // render garbage; close so the client reconnects into
                        // a clean snapshot.
                        eprintln!("[conduit] web client lagged by {n} events; closing");
                        break;
                    }
                }
            }
        }
    }

    let _ = socket.send(Message::Close(None)).await;
}
