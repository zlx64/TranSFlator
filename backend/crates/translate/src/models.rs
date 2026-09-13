//! Model discovery: query a provider's public "list models" endpoint so the UI
//! can offer a dropdown of valid model names (extension of FR-11).
//!
//! The API key is used server-side to authenticate the request and is never
//! returned to the browser. Only providers with a stable, public models
//! endpoint are supported (see [`Provider::supports_model_list`]).

use serde_json::Value;

use crate::provider::Provider;

/// Errors from listing a provider's models.
#[derive(Debug, thiserror::Error)]
pub enum ModelsError {
    #[error("no API key set for {0}")]
    MissingApiKey(String),
    #[error("provider request failed: {0}")]
    Request(String),
}

/// How the API key is attached to the models request.
enum Auth {
    /// `Authorization: Bearer <key>` — key required.
    Bearer,
    /// `Authorization: Bearer <key>` — key optional (OpenRouter lists publicly).
    OptionalBearer,
    /// `?key=<key>` query param — key required (Gemini).
    Query,
}

/// The models endpoint for a provider, or `None` if it can't be listed.
fn endpoint(provider: Provider) -> Option<(&'static str, Auth)> {
    match provider {
        Provider::OpenAi => Some(("https://api.openai.com/v1/models", Auth::Bearer)),
        Provider::DeepSeek => Some(("https://api.deepseek.com/models", Auth::Bearer)),
        Provider::Mistral => Some(("https://api.mistral.ai/v1/models", Auth::Bearer)),
        Provider::OpenRouter => Some((
            "https://openrouter.ai/api/v1/models",
            Auth::OptionalBearer,
        )),
        Provider::Gemini => Some((
            "https://generativelanguage.googleapis.com/v1beta/models",
            Auth::Query,
        )),
        Provider::Claude | Provider::Custom => None,
    }
}

fn require_key(api_key: Option<&str>, provider: Provider) -> Result<&str, ModelsError> {
    api_key
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| ModelsError::MissingApiKey(provider.as_str().to_string()))
}

/// List the models a provider offers, for the UI dropdown.
pub async fn list_models(
    client: &reqwest::Client,
    provider: Provider,
    api_key: Option<&str>,
) -> Result<Vec<String>, ModelsError> {
    let (url, auth) = endpoint(provider).ok_or_else(|| {
        ModelsError::Request("provider does not list models".to_string())
    })?;

    let mut req = client.get(url);
    match auth {
        Auth::Bearer => {
            let key = require_key(api_key, provider)?;
            req = req.bearer_auth(key);
        }
        Auth::OptionalBearer => {
            if let Some(k) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
                req = req.bearer_auth(k);
            }
        }
        Auth::Query => {
            let key = require_key(api_key, provider)?;
            req = req.query(&[("key", key)]);
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| ModelsError::Request(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ModelsError::Request(format!(
            "HTTP {status}: {}",
            truncate(&body, 200)
        )));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| ModelsError::Request(e.to_string()))?;
    Ok(parse_models(provider, &json))
}

/// Derive the OpenAI-compatible models URL for a custom server.
///
/// The explicit URL wins. Otherwise the endpoint is used to infer the API
/// prefix (e.g. `/v1/chat/completions` → `/v1/models`). If that is not
/// possible, a `/v1/models` suffix is assumed.
pub fn custom_models_url(
    server: &str,
    endpoint: Option<&str>,
    explicit: Option<&str>,
) -> Option<String> {
    let server = server.trim().trim_end_matches('/');
    if server.is_empty() {
        return None;
    }
    if let Some(explicit) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(explicit.to_string());
    }
    if let Some(ep_raw) = endpoint.map(str::trim).filter(|s| !s.is_empty()) {
        let ep = if ep_raw.starts_with('/') {
            ep_raw.to_string()
        } else {
            format!("/{ep_raw}")
        };
        if let Some(prefix) = ep.strip_suffix("/chat/completions") {
            return Some(format!("{server}{prefix}/models"));
        }
        if let Some(prefix) = ep.strip_suffix("/completions") {
            return Some(format!("{server}{prefix}/models"));
        }
    }
    let path = server_path(server);
    let has_v1 = path
        .split('/')
        .filter(|seg| !seg.is_empty())
        .any(|seg| seg == "v1");
    if has_v1 {
        Some(format!("{server}/models"))
    } else {
        Some(format!("{server}/v1/models"))
    }
}

fn server_path(server: &str) -> &str {
    let without_scheme = server.splitn(2, "://").nth(1).unwrap_or(server);
    let path_start = without_scheme
        .find('/')
        .map(|i| i + 1)
        .unwrap_or(without_scheme.len());
    &without_scheme[path_start..]
}

/// List models from an OpenAI-compatible custom server.
pub async fn list_models_custom(
    client: &reqwest::Client,
    models_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, ModelsError> {
    let mut req = client.get(models_url);
    if let Some(key) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| ModelsError::Request(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ModelsError::Request(format!(
            "HTTP {status}: {}",
            truncate(&body, 200)
        )));
    }
    let json: Value = resp
        .json()
        .await
        .map_err(|e| ModelsError::Request(e.to_string()))?;
    Ok(parse_models(Provider::Custom, &json))
}

/// Extract model ids from a provider's models response.
fn parse_models(provider: Provider, json: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Gemini + Mistral nest under "models"; the rest under "data".
    let arr = match provider {
        Provider::Gemini | Provider::Mistral => json.get("models").and_then(Value::as_array),
        _ => json.get("data").and_then(Value::as_array),
    };
    if let Some(items) = arr {
        for item in items {
            // Gemini lists `models/<name>`; the others expose `id`.
            let raw = match provider {
                Provider::Gemini => item.get("name").and_then(Value::as_str),
                _ => item.get("id").and_then(Value::as_str),
            };
            let Some(raw) = raw else { continue };
            let cleaned = raw.trim().strip_prefix("models/").unwrap_or(raw.trim());
            if cleaned.is_empty() {
                continue;
            }
            // Gemini also lists embedding/TTS/image/audio models; only offer
            // text-generation models, which is all the pipeline can use.
            if provider == Provider::Gemini && !is_gemini_text_model(cleaned) {
                continue;
            }
            out.push(cleaned.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whether a Gemini model name looks like a text-generation model.
fn is_gemini_text_model(name: &str) -> bool {
    if !name.starts_with("gemini") {
        return false;
    }
    const NON_TEXT_MARKERS: [&str; 8] = [
        "embedding",
        "tts",
        "image",
        "audio",
        "computer-use",
        "robotics",
        "omni",
        "transcribe",
    ];
    !NON_TEXT_MARKERS.iter().any(|m| name.contains(m))
}

/// Char-boundary-safe truncation for error bodies.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = s.floor_char_boundary(max);
        format!("{}…", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_data_ids() {
        let v = json!({"object":"list","data":[{"id":"gpt-4o"},{"id":"gpt-4o-mini"}]});
        assert_eq!(
            parse_models(Provider::OpenAi, &v),
            vec!["gpt-4o", "gpt-4o-mini"]
        );
    }

    #[test]
    fn gemini_strips_prefix_and_filters_non_text() {
        // Mirrors the live endpoint: embeddings, TTS, image, audio, omni,
        // transcribe and robotics models are all dropped; text models kept.
        let v = json!({"models":[
            {"name":"models/gemini-2.5-flash"},
            {"name":"models/gemini-2.5-flash-preview-tts"},
            {"name":"models/gemini-embedding-001"},
            {"name":"models/gemini-2.5-flash-image"},
            {"name":"models/gemini-2.5-flash-native-audio-latest"},
            {"name":"models/gemini-omni-flash-preview"},
            {"name":"models/gemini-3.5-transcribe"},
            {"name":"models/gemini-robotics-er-2-preview"},
            {"name":"models/gemini-2.5-flash-lite"},
            {"name":"models/gemini-3-flash-preview"},
            {"name":"models/text-embedding-004"}
        ]});
        assert_eq!(
            parse_models(Provider::Gemini, &v),
            vec![
                "gemini-2.5-flash",
                "gemini-2.5-flash-lite",
                "gemini-3-flash-preview"
            ]
        );
    }

    #[test]
    fn mistral_models_ids() {
        let v = json!({"models":[{"id":"mistral-large-latest"},{"id":"mistral-small-latest"}]});
        assert_eq!(
            parse_models(Provider::Mistral, &v),
            vec!["mistral-large-latest", "mistral-small-latest"]
        );
    }

    #[test]
    fn openrouter_data_ids_sorted_deduped() {
        let v = json!({"data":[
            {"id":"openai/gpt-4o"},
            {"id":"google/gemini-2.5-flash"},
            {"id":"openai/gpt-4o"}
        ]});
        assert_eq!(
            parse_models(Provider::OpenRouter, &v),
            vec!["google/gemini-2.5-flash", "openai/gpt-4o"]
        );
    }

    #[test]
    fn deepseek_data_ids() {
        let v = json!({"object":"list","data":[{"id":"deepseek-chat"},{"id":"deepseek-reasoner"}]});
        assert_eq!(
            parse_models(Provider::DeepSeek, &v),
            vec!["deepseek-chat", "deepseek-reasoner"]
        );
    }

    #[test]
    fn empty_or_malformed_yields_no_models() {
        assert!(parse_models(Provider::OpenAi, &json!({})).is_empty());
        assert!(parse_models(Provider::Mistral, &json!({"models":{}})).is_empty());
        assert!(parse_models(Provider::Gemini, &json!({"models":[] })).is_empty());
    }

    #[test]
    fn truncate_is_char_boundary_safe() {
        // 3-byte UTF-8 chars; a raw byte slice at 10 would panic without this.
        let s = "аааааааааааааааааа"; // 18 cyrillic chars (36 bytes)
        let t = truncate(s, 10);
        assert!(t.len() <= 10 + 4); // 10 bytes + the ellipsis
        assert!(t.ends_with('…'));
    }

    #[test]
    fn custom_models_url_defaults_to_v1_models() {
        assert_eq!(
            custom_models_url("http://localhost:11434", None, None),
            Some("http://localhost:11434/v1/models".to_string())
        );
    }

    #[test]
    fn custom_models_url_uses_v1_path_from_server() {
        assert_eq!(
            custom_models_url("http://localhost:11434/v1", None, None),
            Some("http://localhost:11434/v1/models".to_string())
        );
    }

    #[test]
    fn custom_models_url_derives_from_chat_endpoint() {
        assert_eq!(
            custom_models_url(
                "http://localhost:11434",
                Some("/v1/chat/completions"),
                None
            ),
            Some("http://localhost:11434/v1/models".to_string())
        );
        assert_eq!(
            custom_models_url(
                "http://localhost:11434/api/v1",
                Some("/chat/completions"),
                None
            ),
            Some("http://localhost:11434/api/v1/models".to_string())
        );
    }

    #[test]
    fn custom_models_url_explicit_wins() {
        assert_eq!(
            custom_models_url(
                "http://localhost:11434",
                Some("/v1/chat/completions"),
                Some("http://localhost:11434/custom/models")
            ),
            Some("http://localhost:11434/custom/models".to_string())
        );
    }

    #[test]
    fn custom_parse_uses_openai_data_shape() {
        let v = json!({"object":"list","data":[{"id":"llama3.1"},{"id":"mistral"}]});
        assert_eq!(
            parse_models(Provider::Custom, &v),
            vec!["llama3.1", "mistral"]
        );
    }
}
