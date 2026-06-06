//! Shared backend bridge for the GUI spike.
//!
//! Owns the tokio runtime and Conduit's `core`, exposing a thread-safe command
//! sink + event subscription that any GUI front-end (GPUI, Iced) can drive.
//! This makes the "reuse ~95% of the backend" claim concrete: the GUI never
//! touches PTYs, git, SSH, or persistence — only `Command` in and `Event` out.

use std::future::Future;

use base64::Engine as _;
use tokio::runtime::Runtime;
pub use tokio::sync::broadcast;

pub use conduit_core::{spawn_core, CoreHandle};
pub use protocol::{self, Command, Event};

/// Sandbox persistence under `workspaces.gui-spike.json` so the spike never
/// touches the user's real Conduit workspace list (see `persist_file()` in core).
const SESSION_NAME: &str = "gui-spike";

pub struct Backend {
    handle: CoreHandle,
    rt: Runtime,
}

impl Backend {
    /// Build a multi-thread runtime and spawn Conduit's core onto it.
    pub fn start() -> Self {
        if std::env::var("CONDUIT_SESSION_NAME").is_err() {
            std::env::set_var("CONDUIT_SESSION_NAME", SESSION_NAME);
        }
        // Start from a clean sandbox each run so demo workspaces don't accumulate.
        if let Ok(home) = std::env::var("HOME") {
            let f = std::path::PathBuf::from(home)
                .join(".config/conduit")
                .join(format!("workspaces.{SESSION_NAME}.json"));
            let _ = std::fs::remove_file(f);
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        // spawn_core() calls tokio::spawn internally, so it must run inside the runtime.
        let handle = {
            let _guard = rt.enter();
            spawn_core()
        };
        Self { handle, rt }
    }

    /// Non-blocking, thread-safe command send. Drops (with a log) only if core's
    /// 1024-deep command queue is full — fine for a spike.
    pub fn send(&self, cmd: Command) {
        if let Err(e) = self.handle.cmd_tx.try_send(cmd) {
            eprintln!("[bridge] command dropped: {e}");
        }
    }

    /// Subscribe to the live event broadcast. Each caller gets its own receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.handle.evt_tx.subscribe()
    }

    /// A cloneable handle (Send + Sync) for GUI event loops to stash in a global,
    /// so they can send commands / subscribe without holding the runtime-owning Backend.
    pub fn core_handle(&self) -> CoreHandle {
        self.handle.clone()
    }

    /// Run a future to completion on the bridge runtime (used by the headless smoke test).
    pub fn block_on<F: Future>(&self, fut: F) -> F::Output {
        self.rt.block_on(fut)
    }
}

/// Decode a `TerminalOutput.data_b64` payload to raw PTY bytes (core uses STANDARD).
pub fn decode_b64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .unwrap_or_default()
}

/// Encode raw bytes for `SendTerminalInput.data_b64` (core uses STANDARD).
pub fn encode_b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
