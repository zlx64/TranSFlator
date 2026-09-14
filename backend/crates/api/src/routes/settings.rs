//! Settings endpoints (§10, §11, §13).
//!
//! - `GET /api/settings` — current effective settings; API keys and the auth
//!   token are **write-only** (returned masked, never in plaintext).
//! - `PUT /api/settings` — update any subset. For secrets, an empty string
//!   clears the value; omitting a field leaves it unchanged.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use transflator_config::app_config::OverwriteBehavior;
use transflator_config::secrets::Secrets;
use transflator_config::settings::{api_key_key, keys, SettingsStore};
use transflator_translate::Provider;

use crate::error::ApiError;
use crate::state::AppState;

/// Build the settings payload shared by GET and the PUT response.
async fn build_response(state: &AppState) -> Result<serde_json::Value, ApiError> {
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());
    let eff = settings
        .effective(state.config.as_ref())
        .await
        .map_err(ApiError::db)?;

    let mut providers = Vec::with_capacity(Provider::all().len());
    for p in Provider::all() {
        let stored = settings
            .get_secret(&api_key_key(p.as_str()))
            .await
            .map_err(ApiError::db)?;
        let (key_set, key_masked) = match stored {
            Some(v) if !v.trim().is_empty() => (true, Some(Secrets::mask(&v))),
            _ => (false, None),
        };
        providers.push(serde_json::json!({
            "id": p.as_str(),
            "label": p.label(),
            "needs_key": p.requires_api_key(),
            "key_set": key_set,
            "key_masked": key_masked,
        }));
    }

    let auth_stored = settings
        .get_secret(keys::AUTH_TOKEN)
        .await
        .map_err(ApiError::db)?;
    let auth_token_set = auth_stored.as_ref().is_some_and(|s| !s.trim().is_empty())
        || !state.config.auth_token.trim().is_empty();
    let auth_token_masked = auth_stored
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| Secrets::mask(s));

    let default_provider = settings
        .get(keys::DEFAULT_PROVIDER)
        .await
        .map_err(ApiError::db)?
        .unwrap_or_default();
    let default_model = settings
        .get(keys::DEFAULT_MODEL)
        .await
        .map_err(ApiError::db)?
        .unwrap_or_default();
    let custom_server_url = settings
        .get(keys::CUSTOM_SERVER_URL)
        .await
        .map_err(ApiError::db)?
        .unwrap_or_default();
    let custom_endpoint = settings
        .get(keys::CUSTOM_ENDPOINT)
        .await
        .map_err(ApiError::db)?
        .unwrap_or_default();
    let custom_model = settings
        .get(keys::CUSTOM_MODEL)
        .await
        .map_err(ApiError::db)?
        .unwrap_or_default();
    let custom_models_url = settings
        .get(keys::CUSTOM_MODELS_URL)
        .await
        .map_err(ApiError::db)?
        .unwrap_or_default();
    let custom_chat = settings
        .get(keys::CUSTOM_CHAT)
        .await
        .map_err(ApiError::db)?
        .map(|s| s.trim() != "false")
        .unwrap_or(true);
    let plex_server_url = settings
        .get(keys::PLEX_SERVER_URL)
        .await
        .map_err(ApiError::db)?
        .unwrap_or_default();
    let plex_token_stored = settings
        .get_secret(keys::PLEX_TOKEN)
        .await
        .map_err(ApiError::db)?;
    let plex_token_set = plex_token_stored
        .as_ref()
        .is_some_and(|s| !s.trim().is_empty());
    let plex_token_masked = plex_token_stored
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| Secrets::mask(s));
    let jellyfin_server_url = settings
        .get(keys::JELLYFIN_SERVER_URL)
        .await
        .map_err(ApiError::db)?
        .unwrap_or_default();
    let jellyfin_token_stored = settings
        .get_secret(keys::JELLYFIN_TOKEN)
        .await
        .map_err(ApiError::db)?;
    let jellyfin_token_set = jellyfin_token_stored
        .as_ref()
        .is_some_and(|s| !s.trim().is_empty());
    let jellyfin_token_masked = jellyfin_token_stored
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| Secrets::mask(s));

    Ok(serde_json::json!({
        "providers": providers,
        "default_provider": default_provider,
        "default_model": default_model,
        "default_target_language": eff.default_target_language,
        "output_pattern": eff.output_pattern,
        "overwrite_behavior": eff.overwrite_behavior.as_str(),
        "concurrency": eff.concurrency,
        "auth_token_set": auth_token_set,
        "auth_token_masked": auth_token_masked,
        "custom_server_url": custom_server_url,
        "custom_endpoint": custom_endpoint,
        "custom_model": custom_model,
        "custom_models_url": custom_models_url,
        "custom_chat": custom_chat,
        "plex_server_url": plex_server_url,
        "plex_token_set": plex_token_set,
        "plex_token_masked": plex_token_masked,
        "jellyfin_server_url": jellyfin_server_url,
        "jellyfin_token_set": jellyfin_token_set,
        "jellyfin_token_masked": jellyfin_token_masked,
    }))
}

/// `GET /api/settings`
pub async fn get(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(build_response(&state).await?))
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsBody {
    /// Provider id → new key. Empty string clears; omitted providers are kept.
    #[serde(default)]
    pub api_keys: Option<std::collections::BTreeMap<String, String>>,
    /// Last-selected provider. Empty string clears; omitted keeps it.
    #[serde(default)]
    pub default_provider: Option<String>,
    /// Last-selected model. Empty string clears; omitted keeps it.
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub default_target_language: Option<String>,
    #[serde(default)]
    pub output_pattern: Option<String>,
    #[serde(default)]
    pub overwrite_behavior: Option<String>,
    #[serde(default)]
    pub concurrency: Option<usize>,
    #[serde(default)]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub custom_server_url: Option<String>,
    #[serde(default)]
    pub custom_endpoint: Option<String>,
    #[serde(default)]
    pub custom_model: Option<String>,
    #[serde(default)]
    pub custom_models_url: Option<String>,
    #[serde(default)]
    pub custom_chat: Option<bool>,
    #[serde(default)]
    pub plex_server_url: Option<String>,
    #[serde(default)]
    pub plex_token: Option<String>,
    #[serde(default)]
    pub jellyfin_server_url: Option<String>,
    #[serde(default)]
    pub jellyfin_token: Option<String>,
}

fn validate_custom_url(value: &str, field: &str) -> Result<String, ApiError> {
    let v = value.trim();
    if !(v.starts_with("http://") || v.starts_with("https://")) {
        return Err(ApiError::bad_request(format!(
            "{field} must start with http:// or https://"
        )));
    }
    Ok(v.to_string())
}

fn normalize_custom_endpoint(value: &str) -> String {
    let v = value.trim();
    if v.starts_with('/') {
        v.to_string()
    } else {
        format!("/{v}")
    }
}

/// `PUT /api/settings`
pub async fn put(
    State(state): State<Arc<AppState>>,
    Json(body): Json<UpdateSettingsBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let settings = SettingsStore::new(&state.pool, state.secrets.as_ref());

    if let Some(keys_map) = &body.api_keys {
        for (id, value) in keys_map {
            if Provider::parse(id).is_none() {
                return Err(ApiError::bad_request(format!("unknown provider: {id}")));
            }
            let key = api_key_key(id);
            if value.trim().is_empty() {
                settings.delete(&key).await.map_err(ApiError::db)?;
            } else {
                settings
                    .set_secret(&key, value)
                    .await
                    .map_err(ApiError::db)?;
            }
        }
    }
    if let Some(v) = &body.default_provider {
        if v.trim().is_empty() {
            settings
                .delete(keys::DEFAULT_PROVIDER)
                .await
                .map_err(ApiError::db)?;
        } else if Provider::parse(v).is_none() {
            return Err(ApiError::bad_request(format!("unknown provider: {v}")));
        } else {
            settings
                .set(keys::DEFAULT_PROVIDER, v)
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.default_model {
        if v.trim().is_empty() {
            settings
                .delete(keys::DEFAULT_MODEL)
                .await
                .map_err(ApiError::db)?;
        } else {
            settings
                .set(keys::DEFAULT_MODEL, v.trim())
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.default_target_language {
        if v.trim().is_empty() {
            settings
                .delete(keys::DEFAULT_TARGET_LANGUAGE)
                .await
                .map_err(ApiError::db)?;
        } else {
            settings
                .set(keys::DEFAULT_TARGET_LANGUAGE, v)
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.output_pattern {
        if v.trim().is_empty() {
            settings
                .delete(keys::OUTPUT_PATTERN)
                .await
                .map_err(ApiError::db)?;
        } else {
            settings
                .set(keys::OUTPUT_PATTERN, v)
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.overwrite_behavior {
        let ob = OverwriteBehavior::parse(v)
            .ok_or_else(|| ApiError::bad_request(format!("invalid overwrite_behavior: {v}")))?;
        settings
            .set(keys::OVERWRITE_BEHAVIOR, ob.as_str())
            .await
            .map_err(ApiError::db)?;
    }
    if let Some(c) = body.concurrency {
        if c < 1 {
            return Err(ApiError::bad_request("concurrency must be >= 1"));
        }
        settings
            .set(keys::CONCURRENCY, &c.to_string())
            .await
            .map_err(ApiError::db)?;
    }
    if let Some(v) = &body.auth_token {
        if v.trim().is_empty() {
            settings
                .delete(keys::AUTH_TOKEN)
                .await
                .map_err(ApiError::db)?;
        } else {
            settings
                .set_secret(keys::AUTH_TOKEN, v)
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.custom_server_url {
        if v.trim().is_empty() {
            settings
                .delete(keys::CUSTOM_SERVER_URL)
                .await
                .map_err(ApiError::db)?;
        } else {
            let url = validate_custom_url(v, "custom_server_url")?;
            settings
                .set(keys::CUSTOM_SERVER_URL, &url)
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.custom_endpoint {
        if v.trim().is_empty() {
            settings
                .delete(keys::CUSTOM_ENDPOINT)
                .await
                .map_err(ApiError::db)?;
        } else {
            settings
                .set(keys::CUSTOM_ENDPOINT, &normalize_custom_endpoint(v))
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.custom_model {
        if v.trim().is_empty() {
            settings
                .delete(keys::CUSTOM_MODEL)
                .await
                .map_err(ApiError::db)?;
        } else {
            settings
                .set(keys::CUSTOM_MODEL, v.trim())
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.custom_models_url {
        if v.trim().is_empty() {
            settings
                .delete(keys::CUSTOM_MODELS_URL)
                .await
                .map_err(ApiError::db)?;
        } else {
            let url = validate_custom_url(v, "custom_models_url")?;
            settings
                .set(keys::CUSTOM_MODELS_URL, &url)
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = body.custom_chat {
        settings
            .set(keys::CUSTOM_CHAT, if v { "true" } else { "false" })
            .await
            .map_err(ApiError::db)?;
    }
    if let Some(v) = &body.plex_server_url {
        if v.trim().is_empty() {
            settings
                .delete(keys::PLEX_SERVER_URL)
                .await
                .map_err(ApiError::db)?;
        } else {
            let url = validate_custom_url(v, "plex_server_url")?;
            settings
                .set(keys::PLEX_SERVER_URL, &url)
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.plex_token {
        if v.trim().is_empty() {
            settings
                .delete(keys::PLEX_TOKEN)
                .await
                .map_err(ApiError::db)?;
            settings
                .delete(keys::PLEX_LAST_PING)
                .await
                .map_err(ApiError::db)?;
        } else {
            settings
                .set_secret(keys::PLEX_TOKEN, v)
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.jellyfin_server_url {
        if v.trim().is_empty() {
            settings
                .delete(keys::JELLYFIN_SERVER_URL)
                .await
                .map_err(ApiError::db)?;
        } else {
            let url = validate_custom_url(v, "jellyfin_server_url")?;
            settings
                .set(keys::JELLYFIN_SERVER_URL, &url)
                .await
                .map_err(ApiError::db)?;
        }
    }
    if let Some(v) = &body.jellyfin_token {
        if v.trim().is_empty() {
            settings
                .delete(keys::JELLYFIN_TOKEN)
                .await
                .map_err(ApiError::db)?;
        } else {
            settings
                .set_secret(keys::JELLYFIN_TOKEN, v)
                .await
                .map_err(ApiError::db)?;
        }
    }

    tracing::info!(
        api_keys = body.api_keys.as_ref().map(|m| m.len()).unwrap_or(0),
        auth_token_changed = body.auth_token.is_some(),
        "settings updated"
    );
    Ok(Json(build_response(&state).await?))
}
