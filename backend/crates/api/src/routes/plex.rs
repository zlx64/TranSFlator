//! Plex PIN authentication and server-discovery endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;
use transflator_config::settings::{keys, SettingsStore};
use transflator_jobs::plex_auth;

use crate::error::ApiError;
use crate::state::AppState;

/// `POST /api/settings/plex/connect`
pub async fn connect(
    State(state): State<Arc<AppState>>,
) -> Result<Json<plex_auth::PlexPinStart>, ApiError> {
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
    let start = plex_auth::start_pin(&state.http, &settings)
        .await
        .map_err(|e| ApiError::internal(format!("Plex connect failed: {e}")))?;
    Ok(Json(start))
}

#[derive(Debug, Serialize)]
pub struct PollResponse {
    pub authorized: bool,
    pub token_set: bool,
}

/// `GET /api/settings/plex/connect/:pin_id`
pub async fn poll(
    State(state): State<Arc<AppState>>,
    Path(pin_id): Path<String>,
) -> Result<Json<PollResponse>, ApiError> {
    let pin_id: u64 = pin_id
        .parse()
        .map_err(|_| ApiError::bad_request("invalid Plex PIN id"))?;
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());

    match plex_auth::poll_pin(&state.http, &settings, pin_id).await {
        Ok(Some(token)) => {
            settings
                .set_secret(keys::PLEX_TOKEN, &token)
                .await
                .map_err(ApiError::db)?;
            tracing::info!("plex token saved via PIN authentication");
            Ok(Json(PollResponse {
                authorized: true,
                token_set: true,
            }))
        }
        Ok(None) => Ok(Json(PollResponse {
            authorized: false,
            token_set: false,
        })),
        Err(e) => Err(ApiError::internal(format!("Plex PIN poll failed: {e}"))),
    }
}

/// `GET /api/settings/plex/servers`
pub async fn servers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
    let servers = plex_auth::list_servers(&state.http, &settings)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(serde_json::json!({ "servers": servers })))
}
