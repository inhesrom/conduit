//! Conduit desktop shell: a native OS window (system webview via `wry`/`tao`)
//! pointed at a trusted local Conduit web server.
//!
//! There is no Electron and no bundled browser — we run the same Axum server
//! the `conduit` binary already serves, but in-process on a private loopback
//! port with TLS/auth off, proxying to the user's running session daemons. The
//! web UI is unchanged: it talks to the server same-origin over WebSocket/REST
//! exactly as in a browser, and shows the session chooser on startup.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use conduit_server::DESKTOP_WEB_PORT;
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
/// `session` pins the window to one registered session; `None` shows the
/// session chooser on startup.
pub fn run(session: Option<String>) -> Result<()> {
    // The tokio runtime hosts the local web server on worker threads; the GUI
    // event loop owns the main thread (macOS requires the NSApplication loop to
    // run there).
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Start the trusted local server and wait for the bound port.
    // The spawned server task keeps running on the runtime after this returns.
    let addr: SocketAddr = rt.block_on(async {
        // `desktop attach <name>`: ensure the pinned registered session's daemon
        // is running before the window connects to it.
        if let Some(name) = &session {
            conduit_core::daemon::attach_session(name).await?;
        }
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<SocketAddr>();
        // Fixed loopback port so the webview origin (and its localStorage-backed
        // settings) is stable across launches. `serve_desktop` falls back to an
        // ephemeral port if it's already in use.
        let bind: SocketAddr = ([127, 0, 0, 1], DESKTOP_WEB_PORT).into();
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
    // Same-origin prefix used to tell in-app navigations from external links.
    let origin = format!("http://{addr}");

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Conduit")
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_decorations(false)
        .build(&event_loop)?;

    // Persist localStorage (settings, theme, fonts, tabs) to a stable on-disk
    // data directory instead of an ephemeral per-launch store. Combined with the
    // fixed port above, this keeps the web UI's settings across restarts.
    // `web_context` must outlive the webview; the event loop below diverges, so
    // it lives for the whole process.
    let mut web_context = wry::WebContext::new(Some(desktop_data_dir()));
    let builder = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_url(&url)
        // WebKitGTK (Linux) and WebView2 (Windows) gate clipboard access off by
        // default; the terminal's Ctrl+C-to-copy writes to the clipboard from
        // JS, so it must be enabled. macOS is always enabled.
        .with_clipboard(true)
        // Clickable terminal links use window.open; route those to the OS
        // browser instead of opening a dead in-webview window.
        .with_new_window_req_handler(|target, _features| {
            let _ = open::that(&target);
            wry::NewWindowResponse::Deny
        })
        // Defense in depth: allow same-origin navigations (the app itself) but
        // send any external top-level navigation to the OS browser rather than
        // letting it replace the app. SPA routing uses the History API and does
        // not trip this.
        .with_navigation_handler(move |target| {
            if target.starts_with(&origin) || target == "about:blank" {
                true
            } else {
                let _ = open::that(&target);
                false
            }
        });

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

/// Persistent data directory for the desktop webview (localStorage, cookies).
/// Mirrors the server's `config_dir()` resolution: `~/.config/conduit` (honoring
/// `XDG_CONFIG_HOME`) with a `desktop-webview` subdir kept separate from the
/// TUI/web config files.
fn desktop_data_dir() -> PathBuf {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("conduit")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/conduit")
    } else {
        PathBuf::from(".")
    };
    base.join("desktop-webview")
}
