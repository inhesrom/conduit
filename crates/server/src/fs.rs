//! Read-only directory listing for the web client's "add repository" browser.
//!
//! The TUI browses the local filesystem directly; a web client (possibly
//! remote) can't, so it asks the daemon. This is request/response and
//! per-client, so it's a plain HTTP GET — NOT an Event (those broadcast to
//! every connected client). Lists directories only; never file contents.

use axum::{extract::Query, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
struct Entry {
    name: String,
    path: String,
    is_repo: bool,
}

#[derive(Debug, Serialize)]
struct Listing {
    path: String,
    parent: Option<String>,
    entries: Vec<Entry>,
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

pub async fn list_dir(Query(q): Query<ListQuery>) -> impl IntoResponse {
    let requested = q
        .path
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(home_dir);

    match read_listing(&requested) {
        Ok(listing) => Json(listing).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

fn read_listing(base: &Path) -> std::io::Result<Listing> {
    let canon = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let mut entries = Vec::new();
    for dirent in std::fs::read_dir(&canon)? {
        let dirent = dirent?;
        if !dirent.file_type()?.is_dir() {
            continue;
        }
        let name = dirent.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // hide dotfiles for a clean browser
        }
        let path = dirent.path();
        let is_repo = path.join(".git").exists();
        entries.push(Entry {
            name,
            path: path.to_string_lossy().to_string(),
            is_repo,
        });
    }
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(Listing {
        path: canon.to_string_lossy().to_string(),
        parent: canon.parent().map(|p| p.to_string_lossy().to_string()),
        entries,
    })
}
