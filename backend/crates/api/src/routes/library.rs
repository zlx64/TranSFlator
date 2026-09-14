//! Library browsing endpoints (FR-1/3/4, §4.1).
//!
//! - `GET /api/library/roots`  — configured media roots.
//! - `GET /api/library/tree`   — list a directory (folders + files).
//! - `GET /api/library/search` — recursive filename search within a root.
//!
//! All filesystem access is path-guarded (§6.9) and offloaded to a blocking
//! thread so the async runtime is never blocked on local I/O.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use transflator_media::cache::{CachedMeta, MediaCache};
use transflator_media::library as lib;
use transflator_media::PathGuard;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    /// Media root index (default 0).
    #[serde(default)]
    pub root: usize,
    /// Path relative to the root, using `/` separators (empty = root).
    #[serde(default)]
    pub path: String,
    /// Include non-video files (default false).
    #[serde(default)]
    pub show_all: bool,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub root: usize,
    /// Case-insensitive substring to match against filenames.
    pub q: String,
    #[serde(default)]
    pub show_all: bool,
}

/// `GET /api/library/roots`
pub async fn roots(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let roots = lib::list_roots(&state.guard);
    Ok(Json(serde_json::json!({ "roots": roots })))
}

/// `GET /api/library/tree?root=&path=&show_all=`
pub async fn tree(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TreeQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let guard = state.guard.clone();
    let path = q.path.clone();
    let tree =
        tokio::task::spawn_blocking(move || lib::list_dir(&guard, q.root, &path, q.show_all))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .map_err(|e| ApiError::from_media(&e))?;

    let files = enrich(tree.files, &state.guard, &state.pool, q.root).await;

    Ok(Json(serde_json::json!({
        "root": q.root,
        "path": q.path,
        "folders": tree.folders,
        "files": files,
    })))
}

/// `GET /api/library/search?root=&q=&show_all=`
pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let guard = state.guard.clone();
    let query = q.q.clone();
    let results =
        tokio::task::spawn_blocking(move || lib::search(&guard, q.root, &query, q.show_all))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .map_err(|e| ApiError::from_media(&e))?;

    let results = enrich(results, &state.guard, &state.pool, q.root).await;

    Ok(Json(serde_json::json!({ "results": results })))
}

/// Fill FR-2 metadata (duration/container/stream counts) on video entries from
/// the ffprobe cache. Files that have not been probed yet keep `None` (badges
/// appear once a file is inspected or translated).
async fn enrich(
    files: Vec<lib::Entry>,
    guard: &PathGuard,
    pool: &sqlx::sqlite::SqlitePool,
    root: usize,
) -> Vec<lib::Entry> {
    // Sync part: resolve + stat each video file to build cache keys.
    let keys: Vec<(String, String, u64, i64)> = tokio::task::spawn_blocking({
        let guard = guard.clone();
        let files = files.clone();
        move || {
            files
                .iter()
                .filter(|f| f.is_video)
                .filter_map(|f| {
                    guard.resolve(root, &f.rel_path).ok().and_then(|abs| {
                        std::fs::metadata(&abs).ok().map(|m| {
                            let mtime = m
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0);
                            (
                                f.rel_path.clone(),
                                abs.to_string_lossy().to_string(),
                                m.len(),
                                mtime,
                            )
                        })
                    })
                })
                .collect()
        }
    })
    .await
    .unwrap_or_default();

    if keys.is_empty() {
        return files;
    }

    let cache = MediaCache::new(pool);
    let metas = cache
        .get_many(
            &keys
                .iter()
                .map(|k| (k.1.clone(), k.2, k.3))
                .collect::<Vec<_>>(),
        )
        .await;

    let by_rel: HashMap<String, CachedMeta> = keys
        .iter()
        .filter_map(|k| metas.get(&k.1).map(|m| (k.0.clone(), m.clone())))
        .collect();

    let thumbs: HashMap<String, String> = keys
        .iter()
        .map(|k| {
            (
                k.0.clone(),
                lib::thumbnail_url(root, &k.0, k.2, k.3, 160, 90, 1),
            )
        })
        .collect();

    files
        .into_iter()
        .map(|mut f| {
            if f.is_video {
                f.thumbnail_url = thumbs.get(&f.rel_path).cloned();
            }
            if let Some(m) = by_rel.get(&f.rel_path) {
                f.duration_s = m.duration_s;
                f.container = m.container.clone();
                f.audio_count = Some(m.audio_count);
                f.subtitle_count = Some(m.subtitle_count);
            }
            f
        })
        .collect()
}
