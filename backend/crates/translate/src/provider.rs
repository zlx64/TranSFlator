//! Translation provider model.
//!
//! The provider list mirrors llm-subtrans's own providers 1:1 (FR-11). Each
//! provider maps to a specific Python script under `scripts/` and (for the
//! hosted providers) an environment variable that carries the API key so the
//! key never appears on the command line (D2).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OpenRouter,
    OpenAi,
    Gemini,
    Claude,
    DeepSeek,
    Mistral,
    Custom,
}

impl Provider {
    /// The llm-subtrans script (relative to `LLM_SUBTRANS_HOME`) for this provider.
    pub fn script_name(&self) -> &'static str {
        match self {
            Provider::OpenRouter => "llm-subtrans.py",
            Provider::OpenAi => "gpt-subtrans.py",
            Provider::Gemini => "gemini-subtrans.py",
            Provider::Claude => "claude-subtrans.py",
            Provider::DeepSeek => "deepseek-subtrans.py",
            Provider::Mistral => "mistral-subtrans.py",
            // Custom server reuses the generic script with `-s`/`-e`.
            Provider::Custom => "llm-subtrans.py",
        }
    }

    /// The environment variable llm-subtrans reads for this provider's API key.
    /// `None` for Custom (key passed via `-k`, or absent for local servers).
    pub fn env_key(&self) -> Option<&'static str> {
        match self {
            Provider::OpenRouter => Some("OPENROUTER_API_KEY"),
            Provider::OpenAi => Some("OPENAI_API_KEY"),
            Provider::Gemini => Some("GEMINI_API_KEY"),
            Provider::Claude => Some("ANTHROPIC_API_KEY"),
            Provider::DeepSeek => Some("DEEPSEEK_API_KEY"),
            Provider::Mistral => Some("MISTRAL_API_KEY"),
            Provider::Custom => None,
        }
    }

    /// Whether this provider requires an API key to run.
    pub fn requires_api_key(&self) -> bool {
        !matches!(self, Provider::Custom)
    }

    /// Whether this provider exposes a public model-listing endpoint, so the UI
    /// can offer a dropdown. Claude has no listing API and Custom's server URL
    /// isn't configured in the pipeline, so both fall back to free-text entry.
    pub fn supports_model_list(&self) -> bool {
        !matches!(self, Provider::Claude | Provider::Custom)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::OpenRouter => "openrouter",
            Provider::OpenAi => "openai",
            Provider::Gemini => "gemini",
            Provider::Claude => "claude",
            Provider::DeepSeek => "deepseek",
            Provider::Mistral => "mistral",
            Provider::Custom => "custom",
        }
    }

    /// Human-readable name for the UI.
    pub fn label(&self) -> &'static str {
        match self {
            Provider::OpenRouter => "OpenRouter",
            Provider::OpenAi => "OpenAI",
            Provider::Gemini => "Google Gemini",
            Provider::Claude => "Anthropic Claude",
            Provider::DeepSeek => "DeepSeek",
            Provider::Mistral => "Mistral",
            Provider::Custom => "Custom (OpenAI-compatible)",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Some(Self::OpenRouter),
            "openai" | "gpt" => Some(Self::OpenAi),
            "gemini" => Some(Self::Gemini),
            "claude" | "anthropic" => Some(Self::Claude),
            "deepseek" => Some(Self::DeepSeek),
            "mistral" => Some(Self::Mistral),
            "custom" | "openai-compatible" => Some(Self::Custom),
            _ => None,
        }
    }

    /// All providers, in UI display order.
    pub fn all() -> &'static [Provider] {
        &[
            Provider::OpenAi,
            Provider::Gemini,
            Provider::Claude,
            Provider::DeepSeek,
            Provider::Mistral,
            Provider::OpenRouter,
            Provider::Custom,
        ]
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
