//! Headless smoke test: proves Conduit's `core` is fully reusable behind the
//! bridge with no TUI. It spawns a real PTY via core, runs a command, and
//! streams the output back through the `Event` channel — exactly what a GUI
//! front-end will consume.

use std::time::Duration;

use gui_spike_bridge::protocol::TerminalKind;
use gui_spike_bridge::{decode_b64, Backend, Command, Event};

fn main() {
    let backend = Backend::start();
    let mut rx = backend.subscribe();

    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    println!("[smoke] core started; AddWorkspace at {cwd}");
    backend.send(Command::AddWorkspace {
        name: "gui-spike-smoke".into(),
        path: cwd,
        ssh: None,
    });

    backend.block_on(async {
        let mut ws_id = None;
        let mut started = false;
        let outcome = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match rx.recv().await {
                    Ok(Event::WorkspaceList { items }) => {
                        if let Some(ws) = items.iter().find(|w| w.name == "gui-spike-smoke") {
                            if !started {
                                started = true;
                                ws_id = Some(ws.id);
                                println!("[smoke] workspace id={} -> StartTerminal(shell)", ws.id);
                                backend.send(Command::StartTerminal {
                                    id: ws.id,
                                    kind: TerminalKind::Shell,
                                    tab_id: None,
                                    cmd: vec![
                                        "bash".into(),
                                        "-lc".into(),
                                        "echo HELLO_FROM_CORE; uname -sm; exit 0".into(),
                                    ],
                                });
                            }
                        }
                    }
                    Ok(Event::TerminalOutput { data_b64, .. }) => {
                        print!("{}", String::from_utf8_lossy(&decode_b64(&data_b64)));
                    }
                    Ok(Event::TerminalExited { code, .. }) => {
                        println!("\n[smoke] terminal exited code={code:?}");
                        break;
                    }
                    Ok(Event::Error { message }) => eprintln!("[smoke] core error: {message}"),
                    Ok(_) => {}
                    Err(_) => break, // lagged or channel closed
                }
            }
        })
        .await;
        if outcome.is_err() {
            eprintln!("[smoke] timed out after 10s waiting for terminal output");
        }
        // Tidy the sandboxed workspace entry so reruns start clean.
        if let Some(id) = ws_id {
            backend.send(Command::RemoveWorkspace { id });
        }
    });

    println!("[smoke] done");
}
