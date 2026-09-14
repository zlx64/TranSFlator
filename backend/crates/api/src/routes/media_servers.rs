//! Connection-test and configuration-removal endpoints for Plex/Jellyfin.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use transflator_config::settings::{keys, SettingsStore};
use transflator_jobs::media_server;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct TestConnectionBody {
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestField {
    ServerUrl,
    Token,
    General,
}

#[derive(Debug, Serialize)]
pub struct TestConnectionResponse {
    pub ok: bool,
    pub message: String,
    pub field: TestField,
}

fn plex_field(message: &str) -> TestField {
    if message.contains("server URL") {
        TestField::ServerUrl
    } else if message.contains("token") {
        TestField::Token
    } else {
        TestField::ServerUrl
    }
}

fn jellyfin_field(message: &str) -> TestField {
    if message.contains("server URL") {
        TestField::ServerUrl
    } else if message.contains("API key") || message.contains("token") {
        TestField::Token
    } else {
        TestField::ServerUrl
    }
}

/// `POST /api/settings/plex/test`
pub async fn test_plex(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TestConnectionBody>,
) -> Result<Json<TestConnectionResponse>, ApiError> {
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
    let result = media_server::test_plex(
        &state.http,
        &settings,
        body.server_url.as_deref(),
        body.token.as_deref(),
    )
    .await;

    Ok(Json(match result {
        Ok(message) => TestConnectionResponse {
            ok: true,
            field: TestField::General,
            message,
        },
        Err(e) => {
            let message = e.to_string();
            TestConnectionResponse {
                ok: false,
                field: plex_field(&message),
                message,
            }
        }
    }))
}

/// `DELETE /api/settings/plex`
pub async fn delete_plex(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
    for key in [
        keys::PLEX_SERVER_URL,
        keys::PLEX_TOKEN,
        keys::PLEX_CLIENT_IDENTIFIER,
        keys::PLEX_LAST_PING,
    ] {
        settings.delete(key).await.map_err(ApiError::db)?;
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `POST /api/settings/jellyfin/test`
pub async fn test_jellyfin(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TestConnectionBody>,
) -> Result<Json<TestConnectionResponse>, ApiError> {
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
    let result = media_server::test_jellyfin(
        &state.http,
        &settings,
        body.server_url.as_deref(),
        body.token.as_deref(),
    )
    .await;

    Ok(Json(match result {
        Ok(message) => TestConnectionResponse {
            ok: true,
            field: TestField::General,
            message,
        },
        Err(e) => {
            let message = e.to_string();
            TestConnectionResponse {
                ok: false,
                field: jellyfin_field(&message),
                message,
            }
        }
    }))
}

/// `DELETE /api/settings/jellyfin`
pub async fn delete_jellyfin(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
    for key in [keys::JELLYFIN_SERVER_URL, keys::JELLYFIN_TOKEN] {
        settings.delete(key).await.map_err(ApiError::db)?;
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}
