//! Session registry + socket discovery — the on-disk record of named daemons
//! (`~/.config/conduit/sessions.json`) and their Unix sockets. Shared by the
//! TUI's session commands and the web proxy that attaches to running sessions.

use std::path::{Path, PathBuf};
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

fn config_dir() -> Option<PathBuf> {
    Some(config_base()?.join("conduit"))
}

pub fn session_socket_dir() -> Result<PathBuf> {
    let base = config_base().ok_or_else(|| anyhow!("cannot determine config directory"))?;
    Ok(base.join("conduit").join("sessions"))
}

pub fn session_socket_path(name: &str) -> Result<PathBuf> {
    validate_session_name(name)?;
    Ok(session_socket_dir()?.join(format!("{}.sock", sanitize_session_name(name))))
}

pub fn session_registry_path() -> Option<PathBuf> {
    Some(config_dir()?.join("sessions.json"))
}

/// Return true when `name` is a valid public session slug.
pub fn is_valid_session_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Validate a public session name.
///
/// Session names are intentionally path-safe slugs. Invalid names are rejected
/// instead of sanitized so two user-visible names cannot collide on one socket
/// or state-file stem.
pub fn validate_session_name(name: &str) -> Result<()> {
    if is_valid_session_name(name) {
        Ok(())
    } else {
        Err(anyhow!(
            "invalid session name '{}': use only ASCII letters, digits, '-' and '_'",
            name
        ))
    }
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

/// A registered session's current liveness.
#[derive(Debug, Clone)]
pub enum RegisteredSession {
    Missing,
    Running(SessionEntry),
    Stale(SessionEntry),
}

/// Return a registered session with its current daemon liveness.
pub fn registered_session(name: &str) -> Result<RegisteredSession> {
    validate_session_name(name)?;
    let registry = load_registry()?;
    let Some(entry) = registry.sessions.iter().find(|s| s.name == name).cloned() else {
        return Ok(RegisteredSession::Missing);
    };
    if socket_alive(&entry.socket_path) {
        Ok(RegisteredSession::Running(entry))
    } else {
        Ok(RegisteredSession::Stale(entry))
    }
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
    let args = cmdline.split_whitespace().collect::<Vec<_>>();
    args.iter().any(|arg| *arg == "run-daemon")
        && args
            .windows(2)
            .any(|window| window == ["--session-name", entry.name.as_str()])
}

/// Per-session persisted state lives under `~/.config/conduit/` keyed by the
/// sanitized session name (matching how the core writes them).
fn session_state_path(name: &str, prefix: &str) -> Option<PathBuf> {
    let safe = sanitize_session_name(name);
    Some(config_dir()?.join(format!("{prefix}.{safe}.json")))
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("failed to remove {}", path.display())),
    }
}

const SESSION_STATE_PREFIXES: &[&str] = &["workspaces", "repositories", "foreground_commands"];

/// Outcome of [`remove_session`].
pub struct RemoveOutcome {
    /// A matching registry entry existed and was removed.
    pub removed: bool,
    /// We sent a kill signal to the session's daemon process.
    pub killed: bool,
}

/// Delete a session entirely: stop its daemon (only when the recorded pid still
/// looks like it), delete its socket file, drop it from the registry, and
/// delete its per-session persisted Conduit state. The caller owns any
/// confirmation. Mirrors the TUI `delete` command; reused by the web/
/// desktop server's `DELETE /api/sessions/{name}`.
pub fn remove_session(name: &str) -> Result<RemoveOutcome> {
    validate_session_name(name)?;
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

    remove_file_if_exists(Path::new(&entry.socket_path))?;

    registry.sessions.retain(|s| s.name != name);
    save_registry(&registry)?;

    for prefix in SESSION_STATE_PREFIXES {
        if let Some(path) = session_state_path(name, prefix) {
            remove_file_if_exists(&path)?;
        }
    }

    Ok(RemoveOutcome {
        removed: true,
        killed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvGuard {
        xdg: Option<String>,
        home: Option<String>,
    }

    impl EnvGuard {
        fn set(root: &Path) -> Self {
            let guard = Self {
                xdg: std::env::var("XDG_CONFIG_HOME").ok(),
                home: std::env::var("HOME").ok(),
            };
            std::env::set_var("XDG_CONFIG_HOME", root);
            std::env::set_var("HOME", root);
            guard
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.xdg {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
            match &self.home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn session_name_validation_accepts_only_non_empty_ascii_slugs() {
        assert!(is_valid_session_name("work"));
        assert!(is_valid_session_name("work-1_agent"));

        for name in ["", "work session", "work/session", "café", ".hidden"] {
            assert!(!is_valid_session_name(name), "{name:?}");
            assert!(validate_session_name(name).is_err(), "{name:?}");
        }
    }

    #[test]
    fn session_socket_path_rejects_names_that_would_sanitize_to_collisions() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _lock = env_lock().lock().unwrap();
        let _env = EnvGuard::set(temp.path());

        let valid = session_socket_path("a_b").expect("valid socket path");
        assert!(valid.ends_with("a_b.sock"));
        assert!(session_socket_path("a b").is_err());
    }

    #[test]
    fn remove_session_deletes_registry_socket_and_session_state_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _lock = env_lock().lock().unwrap();
        let _env = EnvGuard::set(temp.path());

        let socket = temp
            .path()
            .join("conduit")
            .join("sessions")
            .join("work.sock");
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        std::fs::write(&socket, b"stale").unwrap();

        let config = temp.path().join("conduit");
        std::fs::create_dir_all(&config).unwrap();
        for prefix in SESSION_STATE_PREFIXES {
            std::fs::write(config.join(format!("{prefix}.work.json")), b"{}").unwrap();
        }
        std::fs::write(config.join("repositories.other.json"), b"{}").unwrap();

        save_registry(&SessionRegistry {
            sessions: vec![SessionEntry {
                name: "work".to_string(),
                socket_path: socket.display().to_string(),
                pid: 9_999_999,
            }],
        })
        .unwrap();

        let outcome = remove_session("work").unwrap();
        assert!(outcome.removed);
        assert!(!outcome.killed);
        assert!(!socket.exists());
        assert!(load_registry().unwrap().sessions.is_empty());
        for prefix in SESSION_STATE_PREFIXES {
            assert!(!config.join(format!("{prefix}.work.json")).exists());
        }
        assert!(config.join("repositories.other.json").exists());
    }
}
