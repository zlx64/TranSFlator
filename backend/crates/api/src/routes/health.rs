//! Health-page log endpoints.
//!
//! - `GET /api/health/logs` — list rolling log files (newest first).
//! - `GET /api/health/logs/:name` — download one log file as an attachment.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, SecondsFormat, Utc};

use crate::error::ApiError;
use crate::state::AppState;

const LOG_PREFIX: &str = "transflator.log";

/// `GET /api/health/logs`
pub async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let logs = list_log_files(&state.logs_dir).await?;
    Ok(Json(serde_json::json!({ "logs": logs })))
}

/// `GET /api/health/logs/:name`
pub async fn download(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let path = resolve_log_path(&state.logs_dir, &name)?;
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| {
            tracing::warn!(name, error = %e, "failed to read log file");
            ApiError::not_found(format!("log file not found: {name}"))
        })?;

    let cd = format!("attachment; filename=\"{}\"", name.replace('"', ""));
    Ok((
        [
            (header::CONTENT_DISPOSITION, cd),
            (header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string()),
            (header::CONTENT_LENGTH, bytes.len().to_string()),
        ],
        bytes,
    ))
}

async fn list_log_files(logs_dir: &std::path::Path) -> Result<Vec<serde_json::Value>, ApiError> {
    let mut rd = tokio::fs::read_dir(logs_dir)
        .await
        .map_err(|e| {
            tracing::warn!(dir = %logs_dir.display(), error = %e, "failed to read logs dir");
            ApiError::internal(format!("failed to read logs dir: {e}"))
        })?;

    let mut files = Vec::new();
    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|e| ApiError::internal(format!("failed to read logs dir: {e}")))?
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        if !name.starts_with(LOG_PREFIX) {
            continue;
        }
        let meta = entry
            .metadata()
            .await
            .map_err(|e| ApiError::internal(format!("failed to stat log file: {e}")))?;
        let modified_at = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| {
                DateTime::<Utc>::from_timestamp(d.as_secs() as i64, 0)
                    .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
            })
            .flatten();
        files.push(serde_json::json!({
            "name": name,
            "size": meta.len(),
            "modified_at": modified_at,
        }));
    }

    files.sort_by(|a, b| {
        b["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(a["name"].as_str().unwrap_or_default())
    });
    Ok(files)
}

fn resolve_log_path(logs_dir: &std::path::Path, name: &str) -> Result<PathBuf, ApiError> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name == "."
        || name == ".."
        || !name.starts_with(LOG_PREFIX)
    {
        return Err(ApiError::bad_request("invalid log file name"));
    }
    let candidate = logs_dir.join(name);
    if !candidate.is_file() {
        return Err(ApiError::not_found(format!("log file not found: {name}")));
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_log_path_rejects_unsafe_or_unrelated_names() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_log_path(dir.path(), "").is_err());
        assert!(resolve_log_path(dir.path(), "..").is_err());
        assert!(resolve_log_path(dir.path(), "../transflator.log.2024-01-01").is_err());
        assert!(resolve_log_path(dir.path(), "other.log").is_err());
    }

    #[tokio::test]
    async fn list_log_files_returns_only_log_files_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("transflator.log.2024-01-01"), "a").unwrap();
        std::fs::write(dir.path().join("transflator.log.2024-01-02"), "b").unwrap();
        std::fs::write(dir.path().join("not-a-log.txt"), "c").unwrap();

        let files = list_log_files(dir.path()).await.unwrap();
        let names: Vec<&str> = files
            .iter()
            .map(|v| v["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(names, vec!["transflator.log.2024-01-02", "transflator.log.2024-01-01"]);
    }
}