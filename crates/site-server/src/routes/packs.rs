//! Recognizer asset pack downloads for the stat tracker.
//!
//! `GET /api/tracker/packs` lists `manifest.json`.
//! `GET /api/tracker/packs/{name}` streams one allowlisted file.
//!
//! Auth is the daemon token used by `POST /api/stats/upload`, and it does not
//! write. `X-Content-Type-Options` is not set here: `scuffed-server` adds
//! `nosniff` on every response, and Caddy must not add a second copy.

use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio_util::io::ReaderStream;

use crate::extractors::PackDaemonUser;
use crate::packs::{LoadedPacks, load_packs, resolve_pack_file};
use crate::state::AppState;

/// Load the manifest before auth. Unset, unreadable, or invalid packs are 503.
pub(crate) async fn require_packs(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(dir) = state.packs_dir.as_ref() else {
        return disabled();
    };
    match load_packs(dir, &state.upload_dir, &state.reports_dir).await {
        Ok(loaded) => {
            request.extensions_mut().insert(loaded);
            next.run(request).await
        }
        Err(reason) => {
            tracing::warn!(reason, "recognizer packs unavailable");
            disabled()
        }
    }
}

/// GET /api/tracker/packs
pub async fn list_packs(
    _daemon: PackDaemonUser,
    Extension(loaded): Extension<LoadedPacks>,
) -> Response {
    let body = match serde_json::to_vec(&loaded.entries) {
        Ok(body) => body,
        Err(_) => return internal(),
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "private, no-store")
        .body(Body::from(body))
        .unwrap_or_else(|_| internal())
}

/// GET /api/tracker/packs/{name}
pub async fn download_pack(
    _daemon: PackDaemonUser,
    Extension(loaded): Extension<LoadedPacks>,
    Path(name): Path<String>,
) -> Response {
    let Some(entry) = loaded.entries.iter().find(|entry| entry.name == name) else {
        return not_found();
    };
    let Some(path) = resolve_pack_file(&loaded.root, &entry.name).await else {
        return not_found();
    };
    let meta = match tokio::fs::metadata(&path).await {
        Ok(meta) if meta.is_file() && meta.len() == entry.size => meta,
        Ok(_) => {
            tracing::warn!(
                pack = %entry.name,
                "pack file size does not match the manifest"
            );
            return not_found();
        }
        Err(error) => {
            tracing::warn!(pack = %entry.name, %error, "pack file could not be read");
            return not_found();
        }
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(error) => {
            tracing::warn!(pack = %entry.name, %error, "pack file could not be opened");
            return not_found();
        }
    };
    let disposition = format!("attachment; filename=\"{}\"", entry.name);
    let etag = format!("\"{}\"", entry.sha256);
    let length = meta.len().to_string();
    let headers = [
        (header::CONTENT_TYPE, "application/octet-stream"),
        (header::CACHE_CONTROL, "private, no-store"),
        (header::CONTENT_DISPOSITION, &disposition),
        (header::ETAG, &etag),
        (header::CONTENT_LENGTH, &length),
    ];
    let mut builder = Response::builder().status(StatusCode::OK);
    for (name, value) in headers {
        let Ok(value) = HeaderValue::from_str(value) else {
            return internal();
        };
        builder = builder.header(name, value);
    }
    builder
        .body(Body::from_stream(ReaderStream::new(file)))
        .unwrap_or_else(|_| internal())
}

fn disabled() -> Response {
    json_response(StatusCode::SERVICE_UNAVAILABLE, "packs_disabled")
}

fn not_found() -> Response {
    json_response(StatusCode::NOT_FOUND, "pack_not_found")
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(json!({"error": "Internal error"})),
    )
        .into_response()
}

fn json_response(status: StatusCode, error: &'static str) -> Response {
    let body = serde_json::to_vec(&json!({"error": error})).unwrap_or_else(|_| Vec::new());
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "private, no-store")
        .body(Body::from(body))
        .unwrap_or_else(|_| internal())
}
