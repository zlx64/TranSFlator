//! Job data model (spec §9).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Job lifecycle state (FR-16, §6.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Canceled,
    Interrupted,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "canceled" => Some(Self::Canceled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A persisted translation job (spec §9 `jobs` table).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub source_path: PathBuf,
    pub subtitle_stream_index: Option<usize>,
    pub source_language: Option<String>,
    pub target_language: String,
    pub provider: String,
    pub model: Option<String>,
    pub status: JobStatus,
    pub progress_pct: i32,
    pub output_path: Option<PathBuf>,
    pub project_file_path: Option<PathBuf>,
    /// Uploaded external subtitle (§6.1); when set, probe/extraction are
    /// skipped and this file is translated directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_subtitle_path: Option<PathBuf>,
    /// Retained tail of the job log (FR-16).
    pub log_tail: String,
    pub error_message: Option<String>,
    /// How the (next) run should behave; set by `retry` (FR-18).
    pub retry_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

/// How a retry should behave (FR-18, §6.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryMode {
    /// Fresh re-run of the whole pipeline.
    Rerun,
    /// `--retranslate`: re-translate everything (project file must exist).
    Retranslate,
    /// `--reparse`: reprocess existing responses (project file must exist).
    Reparse,
}

impl RetryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rerun => "rerun",
            Self::Retranslate => "retranslate",
            Self::Reparse => "reparse",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rerun" => Some(Self::Rerun),
            "retranslate" => Some(Self::Retranslate),
            "reparse" => Some(Self::Reparse),
            _ => None,
        }
    }
}

impl Default for RetryMode {
    fn default() -> Self {
        Self::Rerun
    }
}

impl std::fmt::Display for RetryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Fields needed to create a new job (the rest is filled in by the store).
#[derive(Debug, Clone)]
pub struct NewJob {
    pub source_path: PathBuf,
    pub subtitle_stream_index: Option<usize>,
    pub source_language: Option<String>,
    pub target_language: String,
    pub provider: String,
    pub model: Option<String>,
    /// Uploaded external subtitle (§6.1); skips probe/extraction when set.
    pub external_subtitle_path: Option<PathBuf>,
}
