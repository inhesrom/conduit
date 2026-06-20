//! Session proxy: relays a browser WebSocket to a running session daemon's
//! Unix socket. Both speak the same JSON protocol, so this is a pure reframe —
//! WS Text ⇄ length-prefixed frame, no (de)serialization. The daemon replays
//! its history on connect, so the browser gets the session's full live state
//! (including already-running agents) the moment it attaches.

use std::path::PathBuf;

use axum::extract::ws::{Message, WebSocket};
use conduit_core::ipc::{read_frame, write_frame};
use tokio::net::UnixStream;

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
