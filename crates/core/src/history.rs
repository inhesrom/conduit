//! Replayable event history.
//!
//! The daemon records state events and terminal output here so that clients
//! connecting later (TUI attach, web UI) receive a snapshot equivalent to
//! having watched the event stream from the start. Shared under a single
//! mutex so snapshots are atomic with respect to the recorder.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

use protocol::{Event, TerminalKind};

/// Per-workspace keyed state store — keeps only the latest of each state event type.
struct WorkspaceState {
    git: Option<Vec<u8>>,
    attention: Option<Vec<u8>>,
}

/// Keyed state store for non-terminal events. No eviction needed — each workspace
/// stores only the latest of each event type.
struct EventHistory {
    repository_list: Option<Vec<u8>>,
    workspace_list: Option<Vec<u8>>,
    per_workspace: HashMap<Uuid, WorkspaceState>,
}

impl EventHistory {
    fn new() -> Self {
        Self {
            repository_list: None,
            workspace_list: None,
            per_workspace: HashMap::new(),
        }
    }

    fn update(&mut self, evt: &Event, payload: Vec<u8>) {
        match evt {
            Event::RepositoryList { .. } => {
                self.repository_list = Some(payload);
            }
            Event::WorkspaceList { .. } => {
                self.workspace_list = Some(payload);
            }
            Event::WorkspaceGitUpdated { id, .. } => {
                self.per_workspace
                    .entry(*id)
                    .or_insert_with(|| WorkspaceState {
                        git: None,
                        attention: None,
                    })
                    .git = Some(payload);
            }
            Event::WorkspaceAttentionChanged { id, .. } => {
                self.per_workspace
                    .entry(*id)
                    .or_insert_with(|| WorkspaceState {
                        git: None,
                        attention: None,
                    })
                    .attention = Some(payload);
            }
            _ => {}
        }
    }

    fn snapshot(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        // Repositories first so the client populates the sidebar tree before
        // workspaces are slotted into it.
        if let Some(ref frame) = self.repository_list {
            out.push(frame.clone());
        }
        if let Some(ref frame) = self.workspace_list {
            out.push(frame.clone());
        }
        for ws in self.per_workspace.values() {
            if let Some(ref frame) = ws.git {
                out.push(frame.clone());
            }
            if let Some(ref frame) = ws.attention {
                out.push(frame.clone());
            }
        }
        out
    }
}

/// Per-terminal ring buffer — stores raw terminal bytes (base64-decoded) per tab.
struct TerminalBuffer {
    kind: TerminalKind,
    data: Vec<u8>,
}

struct TerminalHistory {
    buffers: HashMap<(Uuid, String), TerminalBuffer>,
}

const TERMINAL_HISTORY_MAX_BYTES: usize = 512 * 1024; // 512 KB per terminal tab

impl TerminalHistory {
    fn new() -> Self {
        Self {
            buffers: HashMap::new(),
        }
    }

    fn append(&mut self, id: Uuid, kind: TerminalKind, tab_id: String, raw_bytes: &[u8]) {
        let entry = self
            .buffers
            .entry((id, tab_id))
            .or_insert_with(|| TerminalBuffer {
                kind,
                data: Vec::new(),
            });
        entry.kind = kind;
        entry.data.extend_from_slice(raw_bytes);
        if entry.data.len() > TERMINAL_HISTORY_MAX_BYTES {
            let excess = entry.data.len() - TERMINAL_HISTORY_MAX_BYTES;
            entry.data.drain(..excess);
        }
    }

    fn reset(&mut self, id: Uuid, tab_id: String) {
        self.buffers.remove(&(id, tab_id));
    }

    /// Emit a TerminalStarted + single TerminalOutput per buffer for replay.
    fn snapshot(&self) -> Vec<Event> {
        let mut out = Vec::new();
        for ((id, tab_id), entry) in &self.buffers {
            if entry.data.is_empty() {
                continue;
            }
            out.push(Event::TerminalStarted {
                id: *id,
                kind: entry.kind,
                tab_id: Some(tab_id.clone()),
            });
            let data_b64 = base64::engine::general_purpose::STANDARD.encode(&entry.data);
            out.push(Event::TerminalOutput {
                id: *id,
                kind: entry.kind,
                tab_id: Some(tab_id.clone()),
                data_b64,
            });
        }
        out
    }
}

/// Combined history shared under a single mutex for atomic snapshots.
pub struct CombinedHistory {
    state: EventHistory,
    terminals: TerminalHistory,
}

impl CombinedHistory {
    pub fn new() -> Self {
        Self {
            state: EventHistory::new(),
            terminals: TerminalHistory::new(),
        }
    }

    /// Record one event into history. State events overwrite their keyed slot;
    /// terminal output appends to the per-tab ring buffer.
    fn record(&mut self, evt: &Event) {
        match evt {
            Event::TerminalOutput {
                id,
                kind,
                tab_id,
                data_b64,
            } => {
                if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(data_b64) {
                    let tab = tab_id.clone().unwrap_or_else(|| "default".to_string());
                    self.terminals.append(*id, *kind, tab, &raw);
                }
            }
            Event::TerminalStarted { id, tab_id, .. } => {
                let tab = tab_id.clone().unwrap_or_else(|| "default".to_string());
                self.terminals.reset(*id, tab);
            }
            Event::RepositoryList { .. }
            | Event::WorkspaceList { .. }
            | Event::WorkspaceGitUpdated { .. }
            | Event::WorkspaceAttentionChanged { .. } => {
                if let Ok(payload) = serde_json::to_vec(evt) {
                    self.state.update(evt, payload);
                }
            }
            // TerminalExited: leave buffer intact (shows last output)
            _ => {}
        }
    }

    /// Ordered JSON payloads for a full replay: repositories, workspaces,
    /// per-workspace git/attention, then terminal buffers.
    pub fn snapshot_payloads(&self) -> Vec<Vec<u8>> {
        let mut out = self.state.snapshot();
        for evt in self.terminals.snapshot() {
            if let Ok(payload) = serde_json::to_vec(&evt) {
                out.push(payload);
            }
        }
        out
    }
}

impl Default for CombinedHistory {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the background task that records replayable events into history.
pub fn spawn_recorder(evt_tx: &broadcast::Sender<Event>, history: Arc<Mutex<CombinedHistory>>) {
    let mut evt_rx = evt_tx.subscribe();
    tokio::spawn(async move {
        loop {
            match evt_rx.recv().await {
                Ok(ref evt) => history.lock().await.record(evt),
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[conduit] event recorder lagged by {n} events");
                    continue;
                }
            }
        }
    });
}

/// Atomically snapshot history and subscribe to the live event stream, so no
/// event is missed or duplicated between the two. Payloads are cloned under
/// the lock but returned for the caller to send AFTER the lock is released —
/// a slow client must never stall the recorder.
pub async fn snapshot_and_subscribe(
    history: &Arc<Mutex<CombinedHistory>>,
    evt_tx: &broadcast::Sender<Event>,
) -> (Vec<Vec<u8>>, broadcast::Receiver<Event>) {
    let combined = history.lock().await;
    let payloads = combined.snapshot_payloads();
    let rx = evt_tx.subscribe();
    drop(combined);
    (payloads, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output_event(id: Uuid, tab: &str, text: &str) -> Event {
        Event::TerminalOutput {
            id,
            kind: TerminalKind::Shell,
            tab_id: Some(tab.to_string()),
            data_b64: base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
        }
    }

    #[test]
    fn snapshot_orders_repositories_before_workspaces() {
        let mut history = CombinedHistory::new();
        history.record(&Event::WorkspaceList { items: vec![] });
        history.record(&Event::RepositoryList { items: vec![] });

        let payloads = history.snapshot_payloads();
        assert_eq!(payloads.len(), 2);
        let first: Event = serde_json::from_slice(&payloads[0]).unwrap();
        assert!(matches!(first, Event::RepositoryList { .. }));
        let second: Event = serde_json::from_slice(&payloads[1]).unwrap();
        assert!(matches!(second, Event::WorkspaceList { .. }));
    }

    #[test]
    fn terminal_output_replays_as_started_plus_buffer() {
        let mut history = CombinedHistory::new();
        let id = Uuid::new_v4();
        history.record(&output_event(id, "1", "hello "));
        history.record(&output_event(id, "1", "world"));

        let payloads = history.snapshot_payloads();
        assert_eq!(payloads.len(), 2);
        let started: Event = serde_json::from_slice(&payloads[0]).unwrap();
        assert!(matches!(started, Event::TerminalStarted { .. }));
        match serde_json::from_slice::<Event>(&payloads[1]).unwrap() {
            Event::TerminalOutput { data_b64, .. } => {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap();
                assert_eq!(raw, b"hello world");
            }
            other => panic!("expected TerminalOutput, got {other:?}"),
        }
    }

    #[test]
    fn terminal_started_resets_buffer() {
        let mut history = CombinedHistory::new();
        let id = Uuid::new_v4();
        history.record(&output_event(id, "1", "stale"));
        history.record(&Event::TerminalStarted {
            id,
            kind: TerminalKind::Shell,
            tab_id: Some("1".to_string()),
        });

        assert!(history.snapshot_payloads().is_empty());
    }

    #[test]
    fn terminal_buffer_is_capped() {
        let mut history = CombinedHistory::new();
        let id = Uuid::new_v4();
        let chunk = "x".repeat(64 * 1024);
        for _ in 0..12 {
            history.record(&output_event(id, "1", &chunk));
        }
        match serde_json::from_slice::<Event>(&history.snapshot_payloads()[1]).unwrap() {
            Event::TerminalOutput { data_b64, .. } => {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap();
                assert_eq!(raw.len(), TERMINAL_HISTORY_MAX_BYTES);
            }
            other => panic!("expected TerminalOutput, got {other:?}"),
        }
    }
}
