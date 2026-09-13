//! Library browsing endpoints (FR-1/3/4, §4.1).
//!
//! - `GET /api/library/roots`  — configured media roots.
//! - `GET /api/library/tree`   — list a directory (folders + files).
//! - `GET /api/library/search` — recursive filename search within a root.
//!
//! All filesystem access is path-guarded (§6.9) and offloaded to a blocking
//! thread so the async runtime is never blocked on local I/O.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use transflator_media::library as lib;

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
    let tree = tokio::task::spawn_blocking(move || {
        lib::list_dir(&guard, q.root, &path, q.show_all)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::from_media(&e))?;

    Ok(Json(serde_json::json!({
        "root": q.root,
        "path": q.path,
        "folders": tree.folders,
        "files": tree.files,
    })))
}

/// `GET /api/library/search?root=&q=&show_all=`
pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let guard = state.guard.clone();
    let query = q.q.clone();
    let results = tokio::task::spawn_blocking(move || {
        lib::search(&guard, q.root, &query, q.show_all)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::from_media(&e))?;

    Ok(Json(serde_json::json!({ "results": results })))
}
