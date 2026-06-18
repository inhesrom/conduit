//! Serves the embedded web UI (web/app/dist) with SPA fallback.
//!
//! rust-embed reads from disk in debug builds and embeds in release, so the
//! production binary is self-contained. Unknown paths fall back to index.html
//! so the client-side router handles deep links.

use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/app/dist/"]
struct Assets;

fn serve(path: &str, cache: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    Some(
        Response::builder()
            .header(header::CONTENT_TYPE, mime.as_ref())
            .header(header::CACHE_CONTROL, cache)
            .body(Body::from(file.data.into_owned()))
            .unwrap(),
    )
}

pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty() {
        // Vite emits content-hashed files under assets/ — safe to cache hard.
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        if let Some(resp) = serve(path, cache) {
            return resp;
        }
    }
    // SPA fallback.
    serve("index.html", "no-cache")
        .unwrap_or_else(|| (StatusCode::NOT_FOUND, "web UI not built").into_response())
}
