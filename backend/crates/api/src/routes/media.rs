//! Media inspection endpoint (FR-6/FR-7, §4.2, §6.7).
//!
//! `GET /api/media/streams?root=&path=&force=` — resolves the file through the
//! path guard, serves a cached ffprobe result when path+size+mtime are
//! unchanged (FR-5), otherwise probes and caches. Returns the `MediaInfo`
//! plus the auto-select/picker hint the UI needs.

use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::extract::{Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use transflator_media::cache::MediaCache;
use transflator_media::classify;
use transflator_media::ffprobe::Ffprobe;
use transflator_media::MediaError;

use crate::error::ApiError;
use crate::state::AppState;

/// Cap on a single download (subtitles are tiny; this bounds memory for a
/// bad-faith request of a large video).
const FILE_MAX_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct StreamsQuery {
    /// Media root index (default 0).
    #[serde(default)]
    pub root: usize,
    /// Path relative to the root, using `/` separators.
    pub path: String,
    /// Bypass the cache and re-probe.
    #[serde(default)]
    pub force: bool,
}

/// Strip the Windows `\\?\` verbatim prefix for user-facing messages.
fn display(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

/// Seconds since the Unix epoch (cache invalidation key, FR-5).
fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `GET /api/media/streams?root=&path=&force=`
pub async fn streams(
    State(state): State<Arc<AppState>>,
    Query(q): Query<StreamsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Guard first (§6.9): unknown roots and traversal fail fast.
    let resolved = state
        .guard
        .resolve(q.root, &q.path)
        .map_err(|e| ApiError::from_media(&MediaError::from(e)))?;

    // stat() is a single syscall — no need to hop to the blocking pool.
    // `resolve` tolerates a missing final component (output-file use case),
    // so a missing input file surfaces here as NotFound → 404.
    let meta = match std::fs::metadata(&resolved) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::from_media(&MediaError::NotFound(
                display(&resolved),
            )))
        }
        Err(e) => return Err(ApiError::from_media(&MediaError::Io(e))),
    };
    if !meta.is_file() {
        return Err(ApiError::from_media(&MediaError::NotADirectory(
            display(&resolved),
        )));
    }
    let size = meta.len();
    let mtime = mtime_secs(&meta);

    let cache = MediaCache::new(&state.pool);
    let cached = if q.force {
        None
    } else {
        cache.get(&resolved.to_string_lossy(), size, mtime).await
    };

    let info = match cached {
        Some(info) => info,
        None => {
            let probe = Ffprobe::new(
                &state.config.ffprobe_bin,
                state.config.ffprobe_timeout_secs,
            );
            let probed = probe
                .probe(&resolved)
                .await
                .map_err(|e| ApiError::from_media(&e))?;
            // Cache write failures are non-fatal: serve the fresh result.
            let _ = cache.put(&probed, size, mtime).await;
            probed
        }
    };

    let selection = classify::select_subtitle(&info.streams);
    let preferred = classify::preferred_subtitle_index(&info.streams);

    Ok(Json(serde_json::json!({
        "info": info,
        "selection": selection,
        "preferred": preferred,
    })))
}

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    /// Media root index (default 0).
    #[serde(default)]
    pub root: usize,
    /// Path relative to the root, using `/` separators.
    pub path: String,
}

/// Serve a path-guarded file as a download (`Content-Disposition: attachment`).
/// Shared by the generic `GET /api/media/file` route and the per-job download
/// shortcut. The path is re-resolved through the guard (defense in depth).
pub async fn serve_file(
    state: &AppState,
    root: usize,
    rel: &str,
) -> Result<impl IntoResponse, ApiError> {
    let resolved = state
        .guard
        .resolve(root, rel)
        .map_err(|e| ApiError::from_media(&MediaError::from(e)))?;

    let meta = tokio::fs::metadata(&resolved)
        .await
        .map_err(|e| ApiError::from_media(&MediaError::Io(e)))?;
    if !meta.is_file() {
        return Err(ApiError::from_media(&MediaError::NotFound(
            display(&resolved),
        )));
    }
    let size = meta.len();
    if size > FILE_MAX_BYTES {
        return Err(ApiError::bad_request(
            "file is too large to download through the UI",
        ));
    }
    let bytes = tokio::fs::read(&resolved)
        .await
        .map_err(|e| ApiError::from_media(&MediaError::Io(e)))?;

    let name = resolved
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".into());
    let cd = format!("attachment; filename=\"{}\"", name.replace('"', ""));
    Ok((
        [
            (header::CONTENT_DISPOSITION, cd),
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_LENGTH, size.to_string()),
        ],
        bytes,
    ))
}

/// `GET /api/media/file?root=&path=` — download a path-guarded library file.
pub async fn file(
    State(state): State<Arc<AppState>>,
    Query(q): Query<FileQuery>,
) -> Result<impl IntoResponse, ApiError> {
    serve_file(&state, q.root, &q.path).await
}
