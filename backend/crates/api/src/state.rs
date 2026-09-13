//! Shared application state handed to every handler.

use std::sync::Arc;

use transflator_config::{AppConfig, Secrets};
use transflator_jobs::JobManager;
use transflator_media::PathGuard;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub pool: sqlx::SqlitePool,
    /// Used by the Settings endpoints (Phase 6); the manager holds its own copy.
    #[allow(dead_code)]
    pub secrets: Arc<Secrets>,
    /// Guards every filesystem path the API touches (§6.9).
    pub guard: Arc<PathGuard>,
    /// Job queue + state machine (Phase 4).
    pub manager: Arc<JobManager>,
    /// Shared HTTP client for provider model discovery (FR-11 extension).
    pub http: reqwest::Client,
}
