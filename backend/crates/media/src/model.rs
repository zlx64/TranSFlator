//! Data model for media files and their streams.

use serde::{Deserialize, Serialize};

/// Error type for media operations.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("media root index {0} not found")]
    RootNotFound(usize),
    #[error("path escapes the media root (traversal blocked)")]
    Traversal,
    #[error("path does not exist: {0}")]
    NotFound(String),
    #[error("not a directory: {0}")]
    NotADirectory(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("ffprobe failed: {0}")]
    Ffprobe(String),
    #[error("ffprobe timed out after {0}s")]
    FfprobeTimeout(u64),
    #[error("ffmpeg extraction failed: {0}")]
    Ffmpeg(String),
    #[error("ffmpeg timed out after {0}s")]
    FfmpegTimeout(u64),
    #[error("no subtitle stream found in {0}")]
    NoSubtitles(String),
    #[error("all subtitle streams are image-based (OCR required): {0}")]
    ImageOnly(String),
    #[error("unsupported subtitle codec: {0}")]
    UnsupportedCodec(String),
    #[error("encoding error: {0}")]
    Encoding(String),
    #[error("insufficient disk space: {0}")]
    DiskSpace(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Kind of a media stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Unknown,
}

/// Whether a subtitle stream is text-based (translatable) or image-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleKind {
    Text,
    Image,
}

/// A single media stream as reported by ffprobe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stream {
    /// Overall stream index in the file (0-based, as reported by ffprobe).
    pub index: usize,
    pub kind: StreamKind,
    /// Codec name (e.g. `h264`, `aac`, `subrip`, `hdmv_pgs_subtitle`).
    pub codec: String,
    /// Language tag (ISO 639-2) from stream metadata, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Stream title, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub default: bool,
    pub forced: bool,
    /// For subtitle streams: text vs image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_kind: Option<SubtitleKind>,
}

impl Stream {
    /// A human-readable picker label per §6.7, e.g. `#2 — English (SDH, forced)`.
    pub fn display_label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(lang) = &self.language {
            parts.push(lang.clone());
        }
        if let Some(title) = &self.title {
            if !title.trim().is_empty() {
                parts.push(title.clone());
            }
        }
        let mut flags: Vec<&str> = Vec::new();
        if self.default {
            flags.push("default");
        }
        if self.forced {
            flags.push("forced");
        }
        if let Some(k) = self.subtitle_kind {
            flags.push(match k {
                SubtitleKind::Text => "text",
                SubtitleKind::Image => "image — OCR required",
            });
        }
        let mut label = format!("#{}", self.index);
        if !parts.is_empty() {
            label.push_str(" — ");
            label.push_str(&parts.join(", "));
        }
        if !flags.is_empty() {
            label.push_str(" (");
            label.push_str(&flags.join(", "));
            label.push(')');
        }
        label
    }
}

/// Parsed media file information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub path: String,
    pub duration_s: Option<f64>,
    pub container: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub streams: Vec<Stream>,
    pub video_count: usize,
    pub audio_count: usize,
    pub subtitle_count: usize,
}

/// How the UI should present subtitle selection for a file (FR-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selection {
    /// Exactly one text-based subtitle track; auto-selected.
    Auto { stream_index: usize },
    /// Multiple subtitle tracks; show the picker.
    Picker,
    /// Only image-based subtitle tracks; show the OCR/unsupported notice.
    ImageOnly,
    /// No subtitle tracks at all.
    None,
}
