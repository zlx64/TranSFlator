//! Job event types, forwarded to the API layer for WebSocket fan-out (§10, FR-17).

use serde::{Deserialize, Serialize};

/// A single job event. Tagged `type` field for the JSON wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobEvent {
    /// Status changed (mirrors the `jobs.status` column).
    Status { status: String },
    /// Progress updated (0–100).
    Progress { pct: i32 },
    /// A new line of llm-subtrans output (also retained in `log_tail`).
    Log { line: String },
    /// Job finished successfully.
    Done { output_path: String },
    /// Job failed (or was canceled).
    Failed { message: String },
}

/// Event envelope carrying the job id (spec §10 `/api/jobs/:id/events`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobEventEnvelope {
    pub job_id: String,
    pub event: JobEvent,
}

impl JobEvent {
    /// True if this event represents a terminal job state (the stream can stop).
    pub fn is_terminal(&self) -> bool {
        match self {
            Self::Done { .. } | Self::Failed { .. } => true,
            Self::Status { status } => {
                crate::model::JobStatus::parse(status).is_some_and(|s| s.is_terminal())
            }
            Self::Progress { .. } | Self::Log { .. } => false,
        }
    }
}

impl JobEventEnvelope {
    pub fn new(job_id: impl Into<String>, event: JobEvent) -> Self {
        Self {
            job_id: job_id.into(),
            event,
        }
    }
}
