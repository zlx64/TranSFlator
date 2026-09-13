//! Job endpoints (FR-14/16/17/18, §10).
//!
//! - `POST /api/jobs` — create + enqueue a translation job (fail fast on a
//!   missing API key, §6.6).
//! - `POST /api/jobs/upload` — create a job from an uploaded external
//!   subtitle (§6.1).
//! - `GET /api/jobs?limit=` — list jobs, newest first.
//! - `GET /api/jobs/:id` — fetch one job.
//! - `POST /api/jobs/:id/cancel` — cancel a queued/running job (FR-17).
//! - `POST /api/jobs/:id/retry` — re-run / re-translate / reparse (FR-18).
//! - `GET /api/jobs/:id/events` — per-job WebSocket event stream (FR-16/17).

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use futures_util::StreamExt;
use serde::Deserialize;
use transflator_config::settings::{keys, SettingsStore};
use transflator_jobs::{Job, JobEvent, JobStatus, NewJob, RetryMode};
use transflator_media::MediaError;

use crate::error::ApiError;
use crate::state::AppState;

/// Upload cap for external subtitles (§6.1). Subtitles are small; this bounds
/// memory use for a bad-faith or mistaken upload.
const UPLOAD_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Strip the Windows `\\?\` verbatim prefix for user-facing JSON (same
/// convention as `library::display_path`).
fn strip_verbatim(p: &str) -> &str {
    p.strip_prefix(r"\\?\").unwrap_or(p)
}

/// Serialize a job with display-friendly paths.
fn job_json(job: &Job) -> serde_json::Value {
    let mut v = serde_json::to_value(job).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = v.as_object_mut() {
        for key in ["source_path", "output_path", "project_file_path"] {
            if let Some(v) = obj.get_mut(key) {
                if let Some(s) = v.as_str() {
                    *v = serde_json::Value::String(strip_verbatim(s).to_string());
                }
            }
        }
    }
    v
}

#[derive(Debug, Deserialize)]
pub struct CreateJobBody {
    /// Media root index (default 0).
    #[serde(default)]
    pub root: usize,
    /// Path relative to the root, using `/` separators.
    pub path: String,
    /// Overall subtitle stream index. Omit to auto-select (single text track).
    #[serde(default)]
    pub stream_index: Option<usize>,
    /// Target language. Defaults to `DEFAULT_TARGET_LANGUAGE`.
    #[serde(default)]
    pub target_language: Option<String>,
    /// Provider id (e.g. `openai`, `openrouter`, `custom`).
    pub provider: String,
    /// Model name (provider-specific).
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListJobsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    50
}

/// `POST /api/jobs`
pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateJobBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let resolved = state
        .guard
        .resolve(body.root, &body.path)
        .map_err(|e| ApiError::from_media(&MediaError::from(e)))?;

    // Effective default target language (Settings override the env default).
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
    let default_lang = settings
        .get(keys::DEFAULT_TARGET_LANGUAGE)
        .await
        .map_err(ApiError::db)?
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| state.config.default_target_language.clone());

    let new = NewJob {
        source_path: resolved,
        subtitle_stream_index: body.stream_index,
        source_language: None,
        target_language: body
            .target_language
            .filter(|l| !l.trim().is_empty())
            .unwrap_or(default_lang),
        provider: body.provider,
        model: body.model,
        external_subtitle_path: None,
    };

    let job = state
        .manager
        .create_job(&new)
        .await
        .map_err(|e| ApiError::from_manager(&e))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "job": job_json(&job) }))))
}

/// `POST /api/jobs/upload` (§6.1): create a job from an uploaded external
/// `.srt`/`.ass`/`.vtt` file instead of a stream extracted from the video.
///
/// Multipart fields: `file` (the subtitle), `path` (the video it belongs to,
/// relative to `root` — used for output naming), `root` (default 0),
/// `provider`, `target_language` (optional), `model` (optional).
pub async fn upload(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let mut file: Option<(String, Vec<u8>)> = None;
    let mut root: usize = 0;
    let mut video_path: Option<String> = None;
    let mut target_language: Option<String> = None;
    let mut provider: Option<String> = None;
    let mut model: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("invalid multipart upload: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                let filename = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                // Stream with a byte cap (axum 0.7 has no Multipart::limit).
                let mut bytes: Vec<u8> = Vec::new();
                let mut stream = field;
                while let Some(chunk) = stream.next().await {
                    let chunk =
                        chunk.map_err(|e| ApiError::bad_request(format!("invalid upload file: {e}")))?;
                    bytes.extend_from_slice(&chunk);
                    if bytes.len() > UPLOAD_MAX_BYTES as usize {
                        return Err(ApiError::bad_request(
                            "uploaded file exceeds the 10 MB limit",
                        ));
                    }
                }
                file = Some((filename, bytes));
            }
            "root" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request(format!("invalid field: {e}")))?;
                root = text
                    .parse()
                    .map_err(|_| ApiError::bad_request("root must be a number"))?;
            }
            "path" => {
                video_path = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::bad_request(format!("invalid field: {e}")))?,
                );
            }
            "target_language" => {
                target_language = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::bad_request(format!("invalid field: {e}")))?,
                );
            }
            "provider" => {
                provider = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::bad_request(format!("invalid field: {e}")))?,
                );
            }
            "model" => {
                model = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::bad_request(format!("invalid field: {e}")))?,
                );
            }
            _ => {}
        }
    }

    let (filename, bytes) =
        file.ok_or_else(|| ApiError::bad_request("missing file field"))?;
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(ext.as_str(), "srt" | "ass" | "vtt") {
        return Err(ApiError::bad_request(
            "only .srt, .ass, and .vtt subtitle files can be uploaded",
        ));
    }
    if bytes.is_empty() {
        return Err(ApiError::bad_request("uploaded subtitle file is empty"));
    }
    let video_path = video_path
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("missing path field (the video file)"))?;
    let provider = provider
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("missing provider field"))?;

    // The video must resolve inside a media root (output lands next to it).
    let resolved = state
        .guard
        .resolve(root, &video_path)
        .map_err(|e| ApiError::from_media(&MediaError::from(e)))?;

    // Effective default target language (Settings override the env default).
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
    let default_lang = settings
        .get(keys::DEFAULT_TARGET_LANGUAGE)
        .await
        .map_err(ApiError::db)?
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| state.config.default_target_language.clone());

    // Store the upload in the data dir (RW even when media roots are read-only).
    let uploads = state.config.data_dir.join("uploads");
    tokio::fs::create_dir_all(&uploads)
        .await
        .map_err(|e| ApiError::internal(format!("cannot create uploads dir: {e}")))?;
    let stored = uploads.join(format!("{}.{}", uuid::Uuid::new_v4(), ext));
    if let Err(e) = tokio::fs::write(&stored, &bytes).await {
        return Err(ApiError::internal(format!("cannot store upload: {e}")));
    }

    let new = NewJob {
        source_path: resolved,
        subtitle_stream_index: None,
        source_language: None,
        target_language: target_language
            .filter(|l| !l.trim().is_empty())
            .unwrap_or(default_lang),
        provider,
        model,
        external_subtitle_path: Some(stored.clone()),
    };

    match state.manager.create_job(&new).await {
        Ok(job) => Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({ "job": job_json(&job) })),
        )),
        Err(e) => {
            // Don't leave an orphaned upload behind when the job is rejected.
            let _ = tokio::fs::remove_file(&stored).await;
            Err(ApiError::from_manager(&e))
        }
    }
}

/// `GET /api/jobs?limit=`
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListJobsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let jobs = state
        .manager
        .list(q.limit.min(500))
        .await
        .map_err(|e| ApiError::from_manager(&e))?;
    let jobs: Vec<serde_json::Value> = jobs.iter().map(job_json).collect();
    Ok(Json(serde_json::json!({ "jobs": jobs })))
}

/// `GET /api/jobs/:id`
pub async fn get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let job = state
        .manager
        .get(&id)
        .await
        .map_err(|e| ApiError::from_manager(&e))?;
    Ok(Json(serde_json::json!({ "job": job_json(&job) })))
}

/// `POST /api/jobs/:id/cancel` (FR-17).
pub async fn cancel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .manager
        .cancel_job(&id)
        .await
        .map_err(|e| ApiError::from_manager(&e))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct RetryBody {
    /// `rerun` (default) | `retranslate` | `reparse`.
    #[serde(default)]
    pub mode: String,
}

/// `POST /api/jobs/:id/retry` (FR-18).
pub async fn retry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<RetryBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mode = if body.mode.trim().is_empty() {
        RetryMode::Rerun
    } else {
        RetryMode::parse(&body.mode)
            .ok_or_else(|| ApiError::bad_request(format!("unknown retry mode: {}", body.mode)))?
    };
    let job = state
        .manager
        .retry_job(&id, mode)
        .await
        .map_err(|e| ApiError::from_manager(&e))?;
    Ok(Json(serde_json::json!({ "job": job_json(&job) })))
}

/// `GET /api/jobs/:id/events` (FR-16/17): per-job WebSocket stream.
///
/// On connect it sends a status/progress snapshot (so a reconnecting client
/// resyncs), then streams live [`JobEvent`] frames until the job reaches a
/// terminal state (then closes) or the client disconnects.
pub async fn events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    // Validate the job exists before upgrading (404 otherwise).
    state
        .manager
        .get(&id)
        .await
        .map_err(|e| ApiError::from_manager(&e))?;
    Ok(ws.on_upgrade(move |socket| handle_job_events(state, id, socket)))
}

/// Build the terminal [`JobEvent`] for a finished job, if any (used to give a
/// reconnecting client a clean final frame).
fn terminal_event(job: &Job) -> Option<JobEvent> {
    match job.status {
        JobStatus::Done => Some(JobEvent::Done {
            output_path: job.output_path.as_ref()?.to_string_lossy().to_string(),
        }),
        JobStatus::Failed => Some(JobEvent::Failed {
            message: job.error_message.clone().unwrap_or_else(|| "failed".into()),
        }),
        JobStatus::Canceled => Some(JobEvent::Failed {
            message: "canceled".into(),
        }),
        JobStatus::Interrupted => Some(JobEvent::Failed {
            message: "interrupted (server restarted mid-run)".into(),
        }),
        _ => None,
    }
}

/// Serialize and send one event frame. Returns false if the write failed
/// (client gone).
async fn send_event(socket: &mut WebSocket, event: &JobEvent) -> bool {
    let frame = serde_json::to_string(event).unwrap_or_default();
    socket.send(Message::Text(frame)).await.is_ok()
}

async fn handle_job_events(state: Arc<AppState>, id: String, mut socket: WebSocket) {
    // Subscribe first, then take a fresh snapshot so we never miss the current
    // state (broadcast only delivers events sent after subscription).
    let mut rx = state.manager.subscribe();
    let job = match state.manager.get(&id).await {
        Ok(j) => j,
        Err(_) => return,
    };

    // Snapshot: current status + progress (if any).
    let _ = send_event(&mut socket, &JobEvent::Status { status: job.status.as_str().into() })
        .await;
    if job.progress_pct > 0 {
        let _ = send_event(&mut socket, &JobEvent::Progress { pct: job.progress_pct }).await;
    }

    // If the job is already finished, send the terminal frame and close.
    if job.status.is_terminal() {
        if let Some(ev) = terminal_event(&job) {
            let _ = send_event(&mut socket, &ev).await;
        }
        let _ = socket.close().await;
        return;
    }

    loop {
        tokio::select! {
            res = rx.recv() => {
                match res {
                    Ok(env) if env.job_id == id => {
                        let terminal = env.event.is_terminal();
                        if !send_event(&mut socket, &env.event).await {
                            break;
                        }
                        if terminal {
                            let _ = socket.close().await;
                            break;
                        }
                    }
                    Ok(_) => {} // event for another job
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Missed frames; resync by resending the current status.
                        if let Ok(j) = state.manager.get(&id).await {
                            let _ = send_event(
                                &mut socket,
                                &JobEvent::Status { status: j.status.as_str().into() },
                            )
                            .await;
                            if j.status.is_terminal() {
                                let _ = socket.close().await;
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                // Detect client disconnect so we don't leak the task.
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {} // ignore ping/pong/text from the client
                }
            }
        }
    }
}
