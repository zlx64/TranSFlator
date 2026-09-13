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
use transflator_media::ffmpeg::Ffmpeg;
use transflator_media::ffprobe::Ffprobe;
use transflator_media::library as lib;
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
        Some(info) => {
            tracing::debug!(path = %resolved.display(), "media streams served from cache");
            info
        }
        None => {
            tracing::info!(path = %resolved.display(), "probing media file");
            let probe = Ffprobe::new(
                &state.config.ffprobe_bin,
                state.config.ffprobe_timeout_secs,
            );
            let probed = probe
                .probe(&resolved)
                .await
                .map_err(|e| {
                    tracing::error!(path = %resolved.display(), error = %e, "media probe failed");
                    ApiError::from_media(&e)
                })?;
            tracing::info!(
                path = %resolved.display(),
                subtitles = probed.subtitle_count,
                "media probe completed"
            );
            // Cache write failures are non-fatal: serve the fresh result.
            let _ = cache.put(&probed, size, mtime).await;
            probed
        }
    };

    let selection = classify::select_subtitle(&info.streams);
    let preferred = classify::preferred_subtitle_index(&info.streams);
    let thumbnail_url = lib::thumbnail_url(q.root, &q.path, size, mtime, 480, 270, 1);

    Ok(Json(serde_json::json!({
        "info": info,
        "selection": selection,
        "preferred": preferred,
        "thumbnail_url": thumbnail_url,
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

#[derive(Debug, Deserialize)]
pub struct ThumbnailQuery {
    /// Media root index (default 0).
    #[serde(default)]
    pub root: usize,
    /// Path relative to the root, using `/` separators.
    pub path: String,
    /// Thumbnail width in pixels (clamped to 16..=512).
    #[serde(default = "default_thumb_width")]
    pub width: u32,
    /// Thumbnail height in pixels (clamped to 16..=512).
    #[serde(default = "default_thumb_height")]
    pub height: u32,
    /// Seek position for the thumbnail frame.
    #[serde(default = "default_thumb_at")]
    pub at: u64,
    /// Content signature supplied by the library listing.
    #[serde(default)]
    pub sig: String,
}

fn default_thumb_width() -> u32 {
    160
}

fn default_thumb_height() -> u32 {
    90
}

fn default_thumb_at() -> u64 {
    1
}

/// `GET /api/media/thumbnail?root=&path=&width=&height=&at=&sig=`
///
/// Returns a JPEG thumbnail with a long-lived immutable cache header. The URL
/// is unique per file revision because `sig` is derived from the file's
/// size + mtime.
pub async fn thumbnail(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ThumbnailQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let width = q.width.clamp(16, 512);
    let height = q.height.clamp(16, 512);
    let at = q.at.min(86_400);
    if q.sig.trim().is_empty() {
        return Err(ApiError::bad_request("missing thumbnail signature"));
    }

    let resolved = state
        .guard
        .resolve(q.root, &q.path)
        .map_err(|e| ApiError::from_media(&MediaError::from(e)))?;

    let meta = tokio::fs::metadata(&resolved)
        .await
        .map_err(|e| ApiError::from_media(&MediaError::Io(e)))?;
    if !meta.is_file() {
        return Err(ApiError::from_media(&MediaError::NotFound(
            display(&resolved),
        )));
    }
    let name = resolved
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if !lib::is_video(&name) {
        return Err(ApiError::bad_request("not a video file"));
    }

    let size = meta.len();
    let mtime = mtime_secs(&meta);
    let current_sig = lib::thumbnail_signature(q.root, &q.path, size, mtime);
    if q.sig != current_sig {
        return Err(ApiError::not_found("stale thumbnail signature"));
    }

    let cache_dir = state
        .config
        .data_dir
        .join("thumbnails")
        .join(format!("{width}x{height}"));
    tokio::fs::create_dir_all(&cache_dir)
        .await
        .map_err(|e| ApiError::from_media(&MediaError::Io(e)))?;
    let cache_path = cache_dir.join(format!("{}.jpg", q.sig));

    let cached = tokio::fs::read(&cache_path).await.ok();
    let bytes = match cached {
        Some(b) if !b.is_empty() => b,
        _ => {
            let _ = tokio::fs::remove_file(&cache_path).await;
            tracing::info!(
                path = %resolved.display(),
                width,
                height,
                at,
                "generating thumbnail"
            );
            generate_thumbnail(&state, &resolved, &cache_dir, &cache_path, &q.sig, width, height, at)
                .await?
        }
    };

    Ok(jpeg_response(bytes))
}

async fn generate_thumbnail(
    state: &AppState,
    input: &std::path::Path,
    cache_dir: &std::path::Path,
    cache_path: &std::path::Path,
    sig: &str,
    width: u32,
    height: u32,
    at: u64,
) -> Result<Vec<u8>, ApiError> {
    let ffmpeg = Ffmpeg::new(
        &state.config.ffmpeg_bin,
        state.config.ffmpeg_timeout_secs.min(30),
    );
    let tmp = cache_dir.join(format!("{sig}.{}.tmp", uuid::Uuid::new_v4()));

    let first = ffmpeg.thumbnail(input, &tmp, width, height, at).await;
    let result = match first {
        Ok(_) => Ok(()),
        Err(primary) if at == 0 => Err(primary),
        Err(primary) => {
            // Some containers fail when seeking near/beyond the end; retry at
            // the first frame before surfacing the original error.
            match ffmpeg.thumbnail(input, &tmp, width, height, 0).await {
                Ok(_) => Ok(()),
                Err(_) => Err(primary),
            }
        }
    };

    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&tmp).await;
        tracing::warn!(input = %input.display(), error = %e, "thumbnail generation failed");
        return Err(ApiError::from_media(&e));
    }

    if let Err(e) = tokio::fs::rename(&tmp, cache_path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        tracing::warn!(
            input = %input.display(),
            output = %cache_path.display(),
            error = %e,
            "failed to cache thumbnail"
        );
        return Err(ApiError::from_media(&MediaError::Io(e)));
    }
    tracing::info!(input = %input.display(), output = %cache_path.display(), "thumbnail cached");

    match tokio::fs::read(cache_path).await {
        Ok(b) if !b.is_empty() => Ok(b),
        _ => {
            let _ = tokio::fs::remove_file(cache_path).await;
            Err(ApiError::from_media(&MediaError::Ffmpeg(
                "thumbnail generation produced no image".into(),
            )))
        }
    }
}

fn jpeg_response(bytes: Vec<u8>) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/jpeg".to_string()),
            (header::CONTENT_LENGTH, bytes.len().to_string()),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_string(),
            ),
        ],
        bytes,
    )
}
