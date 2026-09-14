//! API error type: maps domain errors to JSON HTTP responses.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use transflator_jobs::ManagerError;
use transflator_media::MediaError;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: String,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", msg)
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", msg)
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", msg)
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", msg)
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", msg)
    }

    /// Map a database/settings error (anyhow) to a 500 with a redacted message.
    pub fn db(e: impl std::fmt::Display) -> Self {
        Self::internal(format!("settings error: {e}"))
    }

    /// Map a media-domain error to an appropriate HTTP status + code.
    pub fn from_media(e: &MediaError) -> Self {
        match e {
            MediaError::RootNotFound(i) => Self::bad_request(format!("media root {i} not found")),
            MediaError::Traversal => Self::forbidden("path traversal blocked"),
            MediaError::NotFound(p) => Self::not_found(format!("not found: {p}")),
            MediaError::NotADirectory(p) => Self::bad_request(format!("not a directory: {p}")),
            MediaError::PermissionDenied(p) => Self::forbidden(format!("permission denied: {p}")),
            MediaError::NoSubtitles(p) => Self::bad_request(format!("no subtitle tracks in {p}")),
            MediaError::ImageOnly(p) => {
                Self::bad_request(format!("image-only subtitle tracks (OCR required): {p}"))
            }
            MediaError::UnsupportedCodec(c) => {
                Self::bad_request(format!("unsupported subtitle codec: {c}"))
            }
            MediaError::Ffprobe(m) => Self::internal(format!("ffprobe failed: {m}")),
            MediaError::FfprobeTimeout(s) => {
                Self::internal(format!("ffprobe timed out after {s}s"))
            }
            MediaError::Ffmpeg(m) => Self::internal(format!("ffmpeg failed: {m}")),
            MediaError::FfmpegTimeout(s) => Self::internal(format!("ffmpeg timed out after {s}s")),
            MediaError::Encoding(m) => Self::internal(format!("encoding error: {m}")),
            MediaError::DiskSpace(m) => Self::internal(format!("disk space: {m}")),
            MediaError::Io(_) | MediaError::Json(_) => Self::internal(e.to_string()),
        }
    }

    /// Map a job-manager error to an appropriate HTTP status + code.
    pub fn from_manager(e: &ManagerError) -> Self {
        match e {
            ManagerError::NotFound(id) => Self::not_found(format!("job not found: {id}")),
            ManagerError::Invalid(msg) => Self::bad_request(msg),
            ManagerError::MissingApiKey(p) => Self::bad_request(format!(
                "provider {p} requires an API key — set it in Settings first"
            )),
            ManagerError::Store(inner) => match inner {
                transflator_jobs::StoreError::NotFound(id) => {
                    Self::not_found(format!("job not found: {id}"))
                }
                _ => Self::internal(inner.to_string()),
            },
            ManagerError::QueueFull => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "queue_full",
                "job queue is full — try again shortly",
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": self.message,
            "code": self.code,
        });
        (self.status, Json(body)).into_response()
    }
}
