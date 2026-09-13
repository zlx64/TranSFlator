//! Maps job settings to the exact llm-subtrans command line (D1, D2, FR-10).
//!
//! This is the highest-value test target in the project (§15): a wrong flag
//! silently mistranslates or corrupts a run. [`build_command`] is a pure
//! function so it can be exhaustively unit-tested without spawning anything.

use crate::provider::Provider;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Errors from validating a translation request before it starts.
#[derive(Debug, thiserror::Error)]
pub enum TranslateError {
    #[error("provider {0} requires an API key — set it in Settings")]
    MissingApiKey(Provider),
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
}

/// All the settings needed to build a translation command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateOptions {
    pub target_language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Decrypted API key for the selected provider (injected as an env var).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// OpenAI / DeepSeek custom API base (`-b`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    /// Mistral custom server URL (`--server_url`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    /// Custom server address (`-s`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_server: Option<String>,
    /// Custom server endpoint (`-e`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_endpoint: Option<String>,
    /// Custom server chat mode (`--chat`).
    #[serde(default)]
    pub custom_chat: bool,
    /// OpenRouter automatic model selection (`--auto`).
    #[serde(default)]
    pub use_auto_model: bool,
    /// Show name context (`--moviename`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movie_name: Option<String>,
    /// Description context (`--description`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Rate limit in requests/minute (`--ratelimit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<u32>,
    /// Output file (`-o`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
    /// Write a resumable project file (`--project`). Default on.
    #[serde(default = "default_true")]
    pub project: bool,
    /// `--retranslate` (retry: re-translate everything).
    #[serde(default)]
    pub retranslate: bool,
    /// `--reparse` (retry: reprocess existing responses).
    #[serde(default)]
    pub reparse: bool,
    /// `--reload` (retry: reload subtitles from source).
    #[serde(default)]
    pub reload: bool,
    /// `--verbose`.
    #[serde(default)]
    pub verbose: bool,
}

fn default_true() -> bool {
    true
}

impl Default for TranslateOptions {
    fn default() -> Self {
        Self {
            target_language: "English".into(),
            model: None,
            api_key: None,
            api_base: None,
            server_url: None,
            custom_server: None,
            custom_endpoint: None,
            custom_chat: false,
            use_auto_model: false,
            movie_name: None,
            description: None,
            rate_limit: None,
            output: None,
            project: true,
            retranslate: false,
            reparse: false,
            reload: false,
            verbose: false,
        }
    }
}

/// A fully-resolved command to run: script path (relative to the llm-subtrans
/// home), argument list, and environment variables (API keys).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    /// e.g. `scripts/gpt-subtrans.py`
    pub script: String,
    pub args: Vec<String>,
    /// Env vars to set for the child process (API keys live here, not on argv).
    pub env: HashMap<String, String>,
}

/// Validate that a request is runnable (fail fast, §6.6).
pub fn validate(provider: Provider, options: &TranslateOptions) -> Result<(), TranslateError> {
    if provider.requires_api_key() {
        let key = options.api_key.as_deref().unwrap_or("").trim();
        if key.is_empty() {
            return Err(TranslateError::MissingApiKey(provider));
        }
    }
    Ok(())
}

/// Build the llm-subtrans command for a translation job.
pub fn build_command(
    provider: Provider,
    options: &TranslateOptions,
    input: &Path,
) -> CommandSpec {
    let mut args: Vec<String> = Vec::new();
    let mut env: HashMap<String, String> = HashMap::new();

    // Target language (required).
    args.push("-l".into());
    args.push(options.target_language.clone());

    // Output path.
    if let Some(out) = &options.output {
        args.push("-o".into());
        args.push(out.to_string_lossy().to_string());
    }

    // Resumable project file (default on).
    if options.project {
        args.push("--project".into());
    }

    // Context.
    if let Some(m) = &options.movie_name {
        args.push("--moviename".into());
        args.push(m.clone());
    }
    if let Some(d) = &options.description {
        args.push("--description".into());
        args.push(d.clone());
    }
    if let Some(r) = options.rate_limit {
        args.push("--ratelimit".into());
        args.push(r.to_string());
    }

    // Retry / re-run flags.
    if options.retranslate {
        args.push("--retranslate".into());
    }
    if options.reparse {
        args.push("--reparse".into());
    }
    if options.reload {
        args.push("--reload".into());
    }
    if options.verbose {
        args.push("--verbose".into());
    }

    // Provider-specific flags + API key injection.
    match provider {
        Provider::OpenRouter => {
            if options.use_auto_model {
                args.push("--auto".into());
            }
            if let Some(m) = &options.model {
                args.push("-m".into());
                args.push(m.clone());
            }
            if let Some(k) = &options.api_key {
                env.insert("OPENROUTER_API_KEY".into(), k.clone());
            }
        }
        Provider::OpenAi => {
            if let Some(b) = &options.api_base {
                args.push("-b".into());
                args.push(b.clone());
            }
            if let Some(m) = &options.model {
                args.push("-m".into());
                args.push(m.clone());
            }
            if let Some(k) = &options.api_key {
                env.insert("OPENAI_API_KEY".into(), k.clone());
            }
        }
        Provider::Gemini => {
            if let Some(m) = &options.model {
                args.push("-m".into());
                args.push(m.clone());
            }
            if let Some(k) = &options.api_key {
                env.insert("GEMINI_API_KEY".into(), k.clone());
            }
        }
        Provider::Claude => {
            if let Some(m) = &options.model {
                args.push("-m".into());
                args.push(m.clone());
            }
            if let Some(k) = &options.api_key {
                env.insert("ANTHROPIC_API_KEY".into(), k.clone());
            }
        }
        Provider::DeepSeek => {
            if let Some(b) = &options.api_base {
                args.push("-b".into());
                args.push(b.clone());
            }
            if let Some(m) = &options.model {
                args.push("-m".into());
                args.push(m.clone());
            }
            if let Some(k) = &options.api_key {
                env.insert("DEEPSEEK_API_KEY".into(), k.clone());
            }
        }
        Provider::Mistral => {
            if let Some(s) = &options.server_url {
                args.push("--server_url".into());
                args.push(s.clone());
            }
            if let Some(m) = &options.model {
                args.push("-m".into());
                args.push(m.clone());
            }
            if let Some(k) = &options.api_key {
                env.insert("MISTRAL_API_KEY".into(), k.clone());
            }
        }
        Provider::Custom => {
            if let Some(s) = &options.custom_server {
                args.push("-s".into());
                args.push(s.clone());
            }
            if let Some(e) = &options.custom_endpoint {
                args.push("-e".into());
                args.push(e.clone());
            }
            if let Some(m) = &options.model {
                args.push("-m".into());
                args.push(m.clone());
            }
            // Custom has no stable env key; pass on the command line.
            if let Some(k) = &options.api_key {
                args.push("-k".into());
                args.push(k.clone());
            }
            if options.custom_chat {
                args.push("--chat".into());
            }
        }
    }

    // Positional input path goes last.
    args.push(input.to_string_lossy().to_string());

    CommandSpec {
        script: format!("scripts/{}", provider.script_name()),
        args,
        env,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> TranslateOptions {
        TranslateOptions::default()
    }

    #[test]
    fn openai_command() {
        let mut o = opts();
        o.model = Some("gpt-4o-mini".into());
        o.api_key = Some("sk-test".into());
        o.target_language = "Japanese".into();
        o.output = Some(PathBuf::from("/out/sub.ja.srt"));
        o.movie_name = Some("My Show".into());

        let spec = build_command(Provider::OpenAi, &o, Path::new("/in/sub.srt"));
        assert_eq!(spec.script, "scripts/gpt-subtrans.py");
        // API key must be in env, NOT on the command line.
        assert_eq!(spec.env.get("OPENAI_API_KEY").map(String::as_str), Some("sk-test"));
        assert!(!spec.args.iter().any(|a| a == "sk-test"), "key leaked onto argv");
        assert!(!spec.args.iter().any(|a| a == "-k"), "used -k for OpenAI");
        // Key flags present.
        assert!(spec.args.windows(2).any(|w| w == ["-l", "Japanese"]));
        assert!(spec.args.windows(2).any(|w| w == ["-m", "gpt-4o-mini"]));
        assert!(spec.args.windows(2).any(|w| w == ["-o", "/out/sub.ja.srt"]));
        assert!(spec.args.windows(2).any(|w| w == ["--moviename", "My Show"]));
        assert!(spec.args.contains(&"--project".to_string()));
        // Input last.
        assert_eq!(spec.args.last().map(String::as_str), Some("/in/sub.srt"));
    }

    #[test]
    fn openrouter_auto_uses_auto_flag() {
        let mut o = opts();
        o.use_auto_model = true;
        o.api_key = Some("or-key".into());
        let spec = build_command(Provider::OpenRouter, &o, Path::new("in.srt"));
        assert_eq!(spec.script, "scripts/llm-subtrans.py");
        assert!(spec.args.contains(&"--auto".to_string()));
        assert_eq!(spec.env.get("OPENROUTER_API_KEY").map(String::as_str), Some("or-key"));
    }

    #[test]
    fn openrouter_model_uses_m_flag() {
        let mut o = opts();
        o.model = Some("google/gemini-2.5-flash".into());
        o.api_key = Some("or-key".into());
        let spec = build_command(Provider::OpenRouter, &o, Path::new("in.srt"));
        assert!(spec.args.windows(2).any(|w| w == ["-m", "google/gemini-2.5-flash"]));
        assert!(!spec.args.contains(&"--auto".to_string()));
    }

    #[test]
    fn gemini_command() {
        let mut o = opts();
        o.model = Some("gemini-2.5-flash".into());
        o.api_key = Some("gm-key".into());
        let spec = build_command(Provider::Gemini, &o, Path::new("in.srt"));
        assert_eq!(spec.script, "scripts/gemini-subtrans.py");
        assert!(spec.args.windows(2).any(|w| w == ["-m", "gemini-2.5-flash"]));
        assert_eq!(spec.env.get("GEMINI_API_KEY").map(String::as_str), Some("gm-key"));
    }

    #[test]
    fn claude_command() {
        let mut o = opts();
        o.model = Some("claude-3-5-haiku-latest".into());
        o.api_key = Some("an-key".into());
        let spec = build_command(Provider::Claude, &o, Path::new("in.srt"));
        assert_eq!(spec.script, "scripts/claude-subtrans.py");
        assert_eq!(spec.env.get("ANTHROPIC_API_KEY").map(String::as_str), Some("an-key"));
    }

    #[test]
    fn deepseek_command_with_apibase() {
        let mut o = opts();
        o.api_base = Some("https://api.deepseek.com".into());
        o.model = Some("deepseek-chat".into());
        o.api_key = Some("ds-key".into());
        let spec = build_command(Provider::DeepSeek, &o, Path::new("in.srt"));
        assert_eq!(spec.script, "scripts/deepseek-subtrans.py");
        assert!(spec.args.windows(2).any(|w| w == ["-b", "https://api.deepseek.com"]));
        assert_eq!(spec.env.get("DEEPSEEK_API_KEY").map(String::as_str), Some("ds-key"));
    }

    #[test]
    fn mistral_command_with_server_url() {
        let mut o = opts();
        o.server_url = Some("https://custom.mistral.example".into());
        o.model = Some("mistral-large-latest".into());
        o.api_key = Some("ms-key".into());
        let spec = build_command(Provider::Mistral, &o, Path::new("in.srt"));
        assert_eq!(spec.script, "scripts/mistral-subtrans.py");
        assert!(spec.args.windows(2).any(|w| w == ["--server_url", "https://custom.mistral.example"]));
        assert_eq!(spec.env.get("MISTRAL_API_KEY").map(String::as_str), Some("ms-key"));
    }

    #[test]
    fn custom_server_command() {
        let mut o = opts();
        o.custom_server = Some("http://localhost:1234".into());
        o.custom_endpoint = Some("/v1/chat/completions".into());
        o.model = Some("local-model".into());
        o.custom_chat = true;
        // No API key (local server).
        let spec = build_command(Provider::Custom, &o, Path::new("in.srt"));
        assert_eq!(spec.script, "scripts/llm-subtrans.py");
        assert!(spec.args.windows(2).any(|w| w == ["-s", "http://localhost:1234"]));
        assert!(spec.args.windows(2).any(|w| w == ["-e", "/v1/chat/completions"]));
        assert!(spec.args.windows(2).any(|w| w == ["-m", "local-model"]));
        assert!(spec.args.contains(&"--chat".to_string()));
        // No key -> no -k, no env.
        assert!(!spec.args.contains(&"-k".to_string()));
        assert!(spec.env.is_empty());
    }

    #[test]
    fn custom_server_with_key_uses_k_flag() {
        let mut o = opts();
        o.custom_server = Some("http://localhost:1234".into());
        o.api_key = Some("local-key".into());
        let spec = build_command(Provider::Custom, &o, Path::new("in.srt"));
        assert!(spec.args.windows(2).any(|w| w == ["-k", "local-key"]));
    }

    #[test]
    fn retry_flags_mapped() {
        let mut o = opts();
        o.retranslate = true;
        o.reparse = true;
        o.reload = true;
        o.rate_limit = Some(10);
        let spec = build_command(Provider::OpenAi, &o, Path::new("in.srt"));
        assert!(spec.args.contains(&"--retranslate".to_string()));
        assert!(spec.args.contains(&"--reparse".to_string()));
        assert!(spec.args.contains(&"--reload".to_string()));
        assert!(spec.args.windows(2).any(|w| w == ["--ratelimit", "10"]));
    }

    #[test]
    fn project_flag_off_omitted() {
        let mut o = opts();
        o.project = false;
        let spec = build_command(Provider::OpenAi, &o, Path::new("in.srt"));
        assert!(!spec.args.contains(&"--project".to_string()));
    }

    #[test]
    fn validate_missing_key_fails_fast() {
        let o = opts(); // no api_key
        assert!(matches!(
            validate(Provider::OpenAi, &o),
            Err(TranslateError::MissingApiKey(Provider::OpenAi))
        ));
        // Custom does not require a key.
        assert!(validate(Provider::Custom, &o).is_ok());
    }

    #[test]
    fn validate_present_key_ok() {
        let mut o = opts();
        o.api_key = Some("k".into());
        assert!(validate(Provider::OpenAi, &o).is_ok());
        assert!(validate(Provider::Claude, &o).is_ok());
    }

    #[test]
    fn unicode_paths_and_names() {
        let mut o = opts();
        o.movie_name = Some("攻殻機動隊".into());
        o.api_key = Some("k".into());
        let spec = build_command(Provider::OpenAi, &o, Path::new("/media/アニメ/第1話.srt"));
        assert!(spec.args.windows(2).any(|w| w == ["--moviename", "攻殻機動隊"]));
        assert_eq!(spec.args.last().map(String::as_str), Some("/media/アニメ/第1話.srt"));
    }
}
