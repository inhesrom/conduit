//! Password auth + opaque cookie sessions for remote access.
//!
//! A single shared password (argon2id) gates the WebSocket and APIs. The hash
//! is re-read from `web_auth.json` on each auth check (not cached at startup),
//! so `conduit web set-password` takes effect on a running server with no
//! restart. On login the server hands out a random token; it stores only
//! sha256(token) plus an expiry, persisted so a daemon restart doesn't log
//! everyone out. The browser sends the cookie automatically on the WS upgrade,
//! so no auth plumbing leaks into the protocol. When no password is set the
//! server binds localhost only and auth is disabled.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SESSION_TTL_SECS: u64 = 30 * 24 * 3600; // 30-day sliding window
const RATE_WINDOW_SECS: u64 = 300;
const RATE_MAX_FAILURES: u32 = 10;

#[derive(Serialize, Deserialize, Default)]
struct PasswordFile {
    argon2: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct SessionFile {
    sessions: HashMap<String, u64>, // hex(sha256(token)) -> expiry unix secs
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// Hash a password and write it to `path` (0600). Used by `conduit web set-password`.
pub fn set_password(path: &PathBuf, plain: &str) -> anyhow::Result<()> {
    let salt = SaltString::generate(&mut OsRng);
    let phc = Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash failed: {e}"))?
        .to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&PasswordFile { argon2: Some(phc) })?,
    )?;
    set_mode_600(path);
    Ok(())
}

#[cfg(unix)]
fn set_mode_600(path: &PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn set_mode_600(_path: &PathBuf) {}

pub struct Auth {
    auth_path: PathBuf,
    sessions: RwLock<HashMap<String, u64>>,
    rate: RwLock<HashMap<IpAddr, (u32, u64)>>,
    sessions_path: PathBuf,
    pub secure_cookie: bool,
}

impl Auth {
    pub fn load(auth_path: PathBuf, sessions_path: PathBuf, secure_cookie: bool) -> Self {
        let sessions = std::fs::read(&sessions_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<SessionFile>(&b).ok())
            .map(|f| f.sessions)
            .unwrap_or_default();
        Auth {
            auth_path,
            sessions: RwLock::new(sessions),
            rate: RwLock::new(HashMap::new()),
            sessions_path,
            secure_cookie,
        }
    }

    /// The current password hash, re-read from disk on every call. Reading per
    /// attempt (rather than caching at startup) means `conduit web set-password`
    /// is honored immediately by a running server — otherwise a server that was
    /// up when the password changed keeps verifying against the old hash and
    /// rejects the new password as incorrect.
    fn current_password(&self) -> Option<String> {
        std::fs::read(&self.auth_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<PasswordFile>(&b).ok())
            .and_then(|p| p.argon2)
    }

    /// Whether a password is configured. When false, auth is disabled.
    pub fn enabled(&self) -> bool {
        self.current_password().is_some()
    }

    fn persist(&self) {
        let snapshot = SessionFile {
            sessions: self.sessions.read().unwrap().clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&snapshot) {
            let _ = std::fs::write(&self.sessions_path, bytes);
            set_mode_600(&self.sessions_path);
        }
    }

    /// Returns true if the IP is currently rate-limited for login attempts.
    pub fn rate_limited(&self, ip: IpAddr) -> bool {
        let mut rate = self.rate.write().unwrap();
        let n = now();
        let entry = rate.entry(ip).or_insert((0, n));
        if n.saturating_sub(entry.1) > RATE_WINDOW_SECS {
            *entry = (0, n);
        }
        entry.0 >= RATE_MAX_FAILURES
    }

    fn record_failure(&self, ip: IpAddr) {
        let mut rate = self.rate.write().unwrap();
        let n = now();
        let entry = rate.entry(ip).or_insert((0, n));
        if n.saturating_sub(entry.1) > RATE_WINDOW_SECS {
            *entry = (0, n);
        }
        entry.0 += 1;
    }

    /// Verify a password; on success mint a session token (returned raw, to be
    /// set as a cookie). Returns None on mismatch.
    pub fn login(&self, ip: IpAddr, plain: &str) -> Option<String> {
        let phc = self.current_password()?;
        let parsed = PasswordHash::new(&phc).ok()?;
        if Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_err()
        {
            self.record_failure(ip);
            return None;
        }
        let mut raw = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw);
        let token = hex(&raw);
        self.sessions
            .write()
            .unwrap()
            .insert(token_hash(&token), now() + SESSION_TTL_SECS);
        self.persist();
        Some(token)
    }

    /// Validate a token cookie, renewing its sliding expiry. Always true when
    /// auth is disabled.
    pub fn validate(&self, token: Option<&str>) -> bool {
        if !self.enabled() {
            return true;
        }
        let Some(token) = token else { return false };
        let key = token_hash(token);
        let mut sessions = self.sessions.write().unwrap();
        match sessions.get(&key) {
            Some(&expiry) if expiry > now() => {
                sessions.insert(key, now() + SESSION_TTL_SECS);
                true
            }
            Some(_) => {
                sessions.remove(&key);
                false
            }
            None => false,
        }
    }

    pub fn logout(&self, token: Option<&str>) {
        if let Some(token) = token {
            self.sessions.write().unwrap().remove(&token_hash(token));
            self.persist();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        IpAddr::from([127, 0, 0, 1])
    }

    #[test]
    fn password_round_trip_and_sessions() {
        let dir = std::env::temp_dir().join(format!("conduit-auth-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let auth_path = dir.join("web_auth.json");
        let sess_path = dir.join("web_sessions.json");

        set_password(&auth_path, "hunter2").unwrap();
        let auth = Auth::load(auth_path.clone(), sess_path.clone(), false);
        assert!(auth.enabled());

        // Wrong password mints no session; correct one does.
        assert!(auth.login(ip(), "wrong").is_none());
        let token = auth.login(ip(), "hunter2").expect("login");
        assert!(auth.validate(Some(&token)));
        assert!(!auth.validate(Some("garbage")));
        assert!(!auth.validate(None));

        // A reloaded instance honors the persisted session.
        let reloaded = Auth::load(auth_path.clone(), sess_path.clone(), false);
        assert!(reloaded.validate(Some(&token)));

        auth.logout(Some(&token));
        assert!(!auth.validate(Some(&token)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn password_change_takes_effect_without_reload() {
        let dir =
            std::env::temp_dir().join(format!("conduit-auth-reload-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let auth_path = dir.join("web_auth.json");
        let sess_path = dir.join("web_sessions.json");

        set_password(&auth_path, "first").unwrap();
        let auth = Auth::load(auth_path.clone(), sess_path.clone(), false);
        assert!(auth.login(ip(), "first").is_some());

        // Change the password on disk while the same Auth instance stays live —
        // exactly the case a running `conduit web` hits when the user runs
        // `set-password`. The old password must stop working and the new one
        // must work, with no reload of the Auth instance.
        set_password(&auth_path, "second").unwrap();
        assert!(
            auth.login(ip(), "first").is_none(),
            "old password must stop working after set-password"
        );
        assert!(
            auth.login(ip(), "second").is_some(),
            "new password must work without restarting the server"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_auth_allows_everything() {
        let dir = std::env::temp_dir().join(format!("conduit-auth-none-{}", std::process::id()));
        let auth = Auth::load(dir.join("nope.json"), dir.join("sess.json"), false);
        assert!(!auth.enabled());
        assert!(auth.validate(None));
    }
}
