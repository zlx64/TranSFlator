//! Model discovery endpoint (FR-11 extension).
//!
//! `GET /api/models?provider=gemini` — lists the models a provider offers so
//! the UI can render a dropdown. The API key is used server-side to
//! authenticate the provider request and is never returned to the browser.
//! Providers without a public listing endpoint (Claude, Custom) return
//! `supports_list: false` so the UI falls back to free-text entry.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use transflator_config::settings::{api_key_key, keys, SettingsStore};
use transflator_translate::{custom_models_url, list_models, list_models_custom, Provider};

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ModelsQuery {
    pub provider: String,
}

#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    provider: String,
    /// Whether this provider exposes a model-listing endpoint.
    supports_list: bool,
    /// Available model ids (empty when unsupported or when listing failed).
    models: Vec<String>,
    /// Human-readable error when listing failed (e.g. missing key, HTTP error).
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Resolve the API key the same way the job runner does: encrypted settings
/// first, then the process env var.
async fn resolve_key(state: &AppState, provider: Provider) -> Result<Option<String>, ApiError> {
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
    if let Some(key) = settings
        .get_secret(&api_key_key(provider.as_str()))
        .await
        .map_err(ApiError::db)?
    {
        if !key.trim().is_empty() {
            return Ok(Some(key));
        }
    }
    if let Some(env_key) = provider.env_key() {
        if let Ok(v) = std::env::var(env_key) {
            if !v.trim().is_empty() {
                return Ok(Some(v));
            }
        }
    }
    Ok(None)
}

/// `GET /api/models?provider=`
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ModelsQuery>,
) -> Result<Json<ModelsResponse>, ApiError> {
    let provider = Provider::parse(&q.provider)
        .ok_or_else(|| ApiError::bad_request(format!("unknown provider: {}", q.provider)))?;

    if provider == Provider::Custom {
        let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
        let server = settings
            .get(keys::CUSTOM_SERVER_URL)
            .await
            .map_err(ApiError::db)?
            .unwrap_or_default();
        let endpoint = settings
            .get(keys::CUSTOM_ENDPOINT)
            .await
            .map_err(ApiError::db)?;
        let explicit = settings
            .get(keys::CUSTOM_MODELS_URL)
            .await
            .map_err(ApiError::db)?;
        if server.trim().is_empty() {
            return Ok(Json(ModelsResponse {
                provider: provider.as_str().to_string(),
                supports_list: false,
                models: Vec::new(),
                error: None,
            }));
        }
        let key = resolve_key(&state, provider).await?;
        if let Some(url) = custom_models_url(&server, endpoint.as_deref(), explicit.as_deref()) {
            match list_models_custom(&state.http, &url, key.as_deref()).await {
                Ok(models) => {
                    return Ok(Json(ModelsResponse {
                        provider: provider.as_str().to_string(),
                        supports_list: true,
                        models,
                        error: None,
                    }))
                }
                Err(e) => {
                    return Ok(Json(ModelsResponse {
                        provider: provider.as_str().to_string(),
                        supports_list: true,
                        models: Vec::new(),
                        error: Some(e.to_string()),
                    }))
                }
            }
        }
        return Ok(Json(ModelsResponse {
            provider: provider.as_str().to_string(),
            supports_list: false,
            models: Vec::new(),
            error: None,
        }));
    }

    if !provider.supports_model_list() {
        return Ok(Json(ModelsResponse {
            provider: provider.as_str().to_string(),
            supports_list: false,
            models: Vec::new(),
            error: None,
        }));
    }

    let key = resolve_key(&state, provider).await?;
    match list_models(&state.http, provider, key.as_deref()).await {
        Ok(models) => Ok(Json(ModelsResponse {
            provider: provider.as_str().to_string(),
            supports_list: true,
            models,
            error: None,
        })),
        Err(e) => Ok(Json(ModelsResponse {
            provider: provider.as_str().to_string(),
            supports_list: true,
            models: Vec::new(),
            error: Some(e.to_string()),
        })),
    }
}
