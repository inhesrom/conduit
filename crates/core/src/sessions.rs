//! Session registry + socket discovery — the on-disk record of named daemons
//! (`~/.config/conduit/sessions.json`) and their Unix sockets. Shared by the
//! TUI's session commands and the web proxy that attaches to running sessions.

use std::path::PathBuf;
use std::process::Command as OsCommand;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub name: String,
    pub socket_path: String,
    pub pid: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionRegistry {
    pub sessions: Vec<SessionEntry>,
}

fn config_base() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        Some(PathBuf::from(xdg))
    } else {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".config"))
    }
}

pub fn session_socket_dir() -> Result<PathBuf> {
    let base = config_base().ok_or_else(|| anyhow!("cannot determine config directory"))?;
    Ok(base.join("conduit").join("sessions"))
}

pub fn session_socket_path(name: &str) -> Result<PathBuf> {
    Ok(session_socket_dir()?.join(format!("{}.sock", sanitize_session_name(name))))
}

pub fn session_registry_path() -> Option<PathBuf> {
    Some(config_base()?.join("conduit").join("sessions.json"))
}

/// Sanitize a session name for use in file paths (alphanumerics, `-`, `_`).
pub fn sanitize_session_name(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "default".to_string();
    }
    trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn socket_alive(path: &str) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

pub fn load_registry() -> Result<SessionRegistry> {
    let Some(path) = session_registry_path() else {
        return Ok(SessionRegistry::default());
    };
    if !path.exists() {
        return Ok(SessionRegistry::default());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read session registry: {}", path.display()))?;
    Ok(serde_json::from_str::<SessionRegistry>(&raw).unwrap_or_default())
}

pub fn save_registry(registry: &SessionRegistry) -> Result<()> {
    let Some(path) = session_registry_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(registry)?;
    std::fs::write(&path, raw)
        .with_context(|| format!("failed to write session registry: {}", path.display()))?;
    Ok(())
}

/// Registered sessions whose daemon socket currently accepts connections.
pub fn list_running_sessions() -> Vec<SessionEntry> {
    load_registry()
        .map(|r| {
            r.sessions
                .into_iter()
                .filter(|s| socket_alive(&s.socket_path))
                .collect()
        })
        .unwrap_or_default()
}

/// A registered session paired with its current liveness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatus {
    pub name: String,
    /// True when the daemon socket accepts connections; false for a "stale"
    /// entry (e.g. the daemon died on reboot) that can be resurrected.
    pub running: bool,
}

/// All registered sessions, running and stale, each tagged with liveness.
/// Mirrors the TUI `list` command (full registry + per-entry `socket_alive`)
/// so the web/desktop picker can surface — and resurrect — stale sessions.
pub fn list_all_sessions() -> Vec<SessionStatus> {
    load_registry()
        .map(|r| {
            r.sessions
                .into_iter()
                .map(|s| SessionStatus {
                    running: socket_alive(&s.socket_path),
                    name: s.name,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// True when `entry.pid` still looks like this session's daemon — its `ps`
/// cmdline names the `run-daemon` subcommand with a matching `--session-name`.
/// Guards against killing an unrelated process that recycled the pid.
pub fn is_expected_daemon_process(entry: &SessionEntry) -> bool {
    let output = match OsCommand::new("ps")
        .arg("-p")
        .arg(entry.pid.to_string())
        .arg("-o")
        .arg("command=")
        .output()
    {
        Ok(out) => out,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    let cmdline = String::from_utf8_lossy(&output.stdout);
    cmdline.contains("run-daemon") && cmdline.contains(&format!("--session-name {}", entry.name))
}

/// Per-session persisted state lives under `~/.config/conduit/` keyed by the
/// sanitized session name (matching how the core writes them).
fn session_workspaces_persist_path(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let safe = sanitize_session_name(name);
    Some(PathBuf::from(home).join(".config/conduit").join(format!("workspaces.{safe}.json")))
}

fn session_repositories_persist_path(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let safe = sanitize_session_name(name);
    Some(PathBuf::from(home).join(".config/conduit").join(format!("repositories.{safe}.json")))
}

/// Outcome of [`remove_session`].
pub struct RemoveOutcome {
    /// A matching registry entry existed and was removed.
    pub removed: bool,
    /// We sent a kill signal to the session's daemon process.
    pub killed: bool,
}

/// Remove a session entirely: stop its daemon (only when the recorded pid still
/// looks like it), delete its socket file, drop it from the registry, and
/// delete its per-session persisted workspaces/repositories. The caller owns
/// any confirmation. Mirrors the TUI `remove` command; reused by the web/
/// desktop server's `DELETE /api/sessions/{name}`.
pub fn remove_session(name: &str) -> Result<RemoveOutcome> {
    let mut registry = load_registry()?;
    let Some(entry) = registry.sessions.iter().find(|s| s.name == name).cloned() else {
        return Ok(RemoveOutcome {
            removed: false,
            killed: false,
        });
    };

    let killed = is_expected_daemon_process(&entry);
    if killed {
        let _ = OsCommand::new("kill").arg(entry.pid.to_string()).status();
    }

    let _ = std::fs::remove_file(&entry.socket_path);

    registry.sessions.retain(|s| s.name != name);
    save_registry(&registry)?;

    if let Some(path) = session_workspaces_persist_path(name) {
        let _ = std::fs::remove_file(path);
    }
    if let Some(path) = session_repositories_persist_path(name) {
        let _ = std::fs::remove_file(path);
    }

    Ok(RemoveOutcome {
        removed: true,
        killed,
    })
}
