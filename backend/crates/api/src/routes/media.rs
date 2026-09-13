//! Media inspection endpoint (FR-6/FR-7, §4.2, §6.7).
//!
//! `GET /api/media/streams?root=&path=&force=` — resolves the file through the
//! path guard, serves a cached ffprobe result when path+size+mtime are
//! unchanged (FR-5), otherwise probes and caches. Returns the `MediaInfo`
//! plus the auto-select/picker hint the UI needs.

use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use transflator_media::cache::MediaCache;
use transflator_media::classify;
use transflator_media::ffprobe::Ffprobe;
use transflator_media::MediaError;

use crate::error::ApiError;
use crate::state::AppState;

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
