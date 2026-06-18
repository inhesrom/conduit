//! Session registry + socket discovery — the on-disk record of named daemons
//! (`~/.config/conduit/sessions.json`) and their Unix sockets. Shared by the
//! TUI's session commands and the web proxy that attaches to running sessions.

use std::path::PathBuf;

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
