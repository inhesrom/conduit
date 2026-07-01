//! Cross-platform resolution of Conduit's config and home directories.
//!
//! Unix keeps the existing XDG behaviour (`$XDG_CONFIG_HOME` or `~/.config`);
//! Windows uses `%APPDATA%` (the roaming app-data root) with a `%USERPROFILE%`
//! fallback. Callers join `"conduit"` onto [`config_root`] for the app's own
//! config folder, exactly as the per-crate helpers did before.

use std::path::PathBuf;

/// Base directory that Conduit's `conduit/` config folder lives under.
///
/// - Unix: `$XDG_CONFIG_HOME` (when set and non-empty), else `~/.config`.
/// - Windows: `%APPDATA%`, else `%USERPROFILE%\.config`.
pub fn config_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA").filter(|s| !s.is_empty()) {
            return Some(PathBuf::from(appdata));
        }
        std::env::var_os("USERPROFILE")
            .filter(|s| !s.is_empty())
            .map(|h| PathBuf::from(h).join(".config"))
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.trim().is_empty() {
                return Some(PathBuf::from(xdg));
            }
        }
        std::env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty())
            .map(|h| PathBuf::from(h).join(".config"))
    }
}

/// The current user's home directory. Unix: `$HOME`. Windows: `%USERPROFILE%`.
pub fn home() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    }
}
