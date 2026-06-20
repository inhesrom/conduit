//! Verifies the desktop app's embedded server path (`serve_embedded`) spins up
//! an in-process core + web server on an ephemeral loopback port and serves
//! over plain HTTP with no auth/TLS — no session daemon required.

use std::sync::Arc;
use std::time::Duration;

use conduit_core::history::{spawn_recorder, CombinedHistory};
use conduit_server::{serve_embedded, EmbeddedCore};
use tokio::sync::Mutex;

#[tokio::test(flavor = "multi_thread")]
async fn embedded_server_serves_healthz_over_plain_http() {
    // Isolate from the user's real ~/.config/conduit so core spin-up is inert.
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());

    let core = conduit_core::spawn_core();
    let history = Arc::new(Mutex::new(CombinedHistory::new()));
    spawn_recorder(&core.evt_tx, history.clone());
    let embedded = EmbeddedCore { core, history };

    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let bind = ([127, 0, 0, 1], 0).into();
    tokio::spawn(async move {
        let _ = serve_embedded(bind, "desktop".to_string(), embedded, ready_tx).await;
    });

    let addr = tokio::time::timeout(Duration::from_secs(5), ready_rx)
        .await
        .expect("server did not bind in time")
        .expect("ready channel dropped");

    // Plain-HTTP GET /healthz via std sockets (no tokio io-util dependency).
    let resp = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        s.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut buf = String::new();
        s.read_to_string(&mut buf).unwrap();
        buf
    })
    .await
    .unwrap();

    assert!(resp.contains("200 OK"), "expected 200 from /healthz, got:\n{resp}");
    assert!(
        resp.contains("\"name\":\"conduit\""),
        "healthz body missing name field:\n{resp}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn embedded_ws_bridge_replays_snapshot() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());

    let core = conduit_core::spawn_core();
    let history = Arc::new(Mutex::new(CombinedHistory::new()));
    spawn_recorder(&core.evt_tx, history.clone());
    let embedded = EmbeddedCore { core, history };

    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let bind = ([127, 0, 0, 1], 0).into();
    tokio::spawn(async move {
        let _ = serve_embedded(bind, "desktop".to_string(), embedded, ready_tx).await;
    });
    let addr = tokio::time::timeout(Duration::from_secs(5), ready_rx)
        .await
        .expect("server did not bind in time")
        .expect("ready channel dropped");

    // The WS upgrade succeeding proves auth (disabled) lets it through and the
    // embedded bridge started; the first frame proves the snapshot replay path.
    let (mut ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("websocket upgrade failed");

    let first = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("no snapshot frame within timeout")
        .expect("websocket closed before any frame")
        .expect("websocket error");

    let text = match first {
        Message::Text(t) => t.to_string(),
        other => panic!("expected a Text snapshot frame, got: {other:?}"),
    };
    // The bridge must deliver well-formed protocol Events (the core's startup
    // snapshot). The exact first event varies (list vs per-workspace git).
    serde_json::from_str::<protocol::Event>(&text)
        .unwrap_or_else(|e| panic!("first WS frame was not a valid Event ({e}): {text}"));
}
