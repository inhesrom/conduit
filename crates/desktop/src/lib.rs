//! Conduit desktop shell: a native OS window (system webview via `wry`/`tao`)
//! pointed at an in-process Conduit web server.
//!
//! There is no Electron and no bundled browser — we run the same Axum server
//! the `conduit` binary already serves, but in-process on an ephemeral loopback
//! port with TLS/auth off, proxying to the user's running session daemons. The
//! web UI is unchanged: it talks to the server same-origin over WebSocket/REST
//! exactly as in a browser, and shows the session picker on startup.

use std::net::SocketAddr;

use anyhow::{anyhow, Result};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

/// Launch the desktop app. Blocks (runs the GUI event loop) until the window is
/// closed, then exits the process. Must be called on the main thread.
///
/// `session` pins the window to one session (created if missing); `None` shows
/// the session picker on startup.
pub fn run(session: Option<String>) -> Result<()> {
    // The tokio runtime hosts the in-process core + embedded web server on
    // worker threads; the GUI event loop owns the main thread (macOS requires
    // the NSApplication loop to run there).
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Start the core + embedded server and wait for the bound (ephemeral) port.
    // The spawned server task keeps running on the runtime after this returns.
    let addr: SocketAddr = rt.block_on(async {
        // `desktop attach <name>`: ensure the pinned session's daemon is running
        // (created if missing) before the window connects to it.
        if let Some(name) = &session {
            conduit_core::daemon::ensure_session_running(name).await?;
        }
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<SocketAddr>();
        let bind: SocketAddr = ([127, 0, 0, 1], 0).into();
        tokio::spawn(async move {
            if let Err(e) = conduit_server::serve_desktop(bind, session, ready_tx).await {
                eprintln!("[conduit] desktop server exited: {e}");
            }
        });

        // The tao event loop owns the main thread and only exits on window
        // close, so wire terminal signals to quit. The runtime's worker threads
        // keep its signal driver alive while the main thread is in the GTK loop.
        tokio::spawn(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                let mut term = signal(SignalKind::terminate()).ok();
                let ctrl_c = tokio::signal::ctrl_c();
                tokio::select! {
                    _ = ctrl_c => {}
                    _ = async { match term.as_mut() { Some(s) => { s.recv().await; } None => std::future::pending().await } } => {}
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
            std::process::exit(0);
        });

        let addr = ready_rx.await?;
        Ok::<SocketAddr, anyhow::Error>(addr)
    })?;

    let url = format!("http://{addr}/");

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Conduit")
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_decorations(false)
        .build(&event_loop)?;

    let builder = WebViewBuilder::new().with_url(&url);

    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    let _webview = builder.build(&window)?;
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    let _webview = {
        // On Linux the webview is a GTK widget, built into the window's vbox.
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window
            .default_vbox()
            .ok_or_else(|| anyhow!("window has no GTK vbox"))?;
        builder.build_gtk(vbox)?
    };

    // `rt` stays in scope (and thus alive) for the whole process; the event
    // loop below diverges, so it is never dropped while the app is running.
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}
