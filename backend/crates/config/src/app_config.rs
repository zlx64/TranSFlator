//! Process configuration loaded from environment variables with sane defaults.
//!
//! Every variable the app reads is listed here and mirrored in `.env.example`.

use std::path::PathBuf;

/// Overwrite behavior when the computed output file already exists (§6.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverwriteBehavior {
    /// Overwrite the existing file.
    Overwrite,
    /// Append a numeric suffix (`-2`, `-3`, ...) to find a free name.
    #[default]
    Suffix,
    /// Skip the job with a clear message.
    Skip,
}

impl OverwriteBehavior {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "overwrite" => Some(Self::Overwrite),
            "suffix" => Some(Self::Suffix),
            "skip" => Some(Self::Skip),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Overwrite => "overwrite",
            Self::Suffix => "suffix",
            Self::Skip => "skip",
        }
    }
}

/// Fully-resolved application configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Media library root folders to browse (read-only).
    pub media_roots: Vec<PathBuf>,
    /// Path to the SQLite database file.
    pub db_path: PathBuf,
    /// Directory for runtime data (DB, extracted subs, project files).
    pub data_dir: PathBuf,
    /// Secret used to derive the AES key for encrypting API keys at rest.
    pub app_secret: String,
    /// Optional shared access token gating the API/UI (NFR-7). Empty = open.
    pub auth_token: String,
    /// Bind host.
    pub host: String,
    /// Bind port.
    pub port: u16,
    /// Home directory of the bundled llm-subtrans checkout.
    pub llm_subtrans_home: PathBuf,
    /// Python interpreter used to run llm-subtrans scripts.
    pub python_bin: String,
    /// ffmpeg / ffprobe binaries.
    pub ffmpeg_bin: String,
    pub ffprobe_bin: String,
    /// Max concurrent translation jobs (FR-14).
    pub concurrency: usize,
    /// Output naming pattern. Placeholders: `{name}`, `{lang}`, `{ext}`.
    pub output_pattern: String,
    /// What to do when the output file already exists (§6.8).
    pub overwrite_behavior: OverwriteBehavior,
    /// Default target language for new jobs.
    pub default_target_language: String,
    /// Subprocess timeouts.
    pub ffprobe_timeout_secs: u64,
    pub ffmpeg_timeout_secs: u64,
    /// Where the built frontend is served from.
    pub web_root: PathBuf,
    /// Log level filter (tracing).
    pub log_level: String,
}

/// Read a string env var, falling back to `default` when the variable is unset
/// **or** set to an empty/whitespace value (common when a container env passes
/// through an unset host variable).
fn env_str(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => default.to_string(),
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
        .max(1)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Split a roots string into paths, trimming and dropping empty entries.
///
/// `;` is the primary separator on every platform. `:` is also accepted on
/// Unix (PATH-style) but **not** on Windows, where it is the drive-letter
/// separator (e.g. `C:\media`).
fn split_roots(raw: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    let seps: &[char] = &[';'];
    #[cfg(not(windows))]
    let seps: &[char] = &[';', ':'];
    raw.split(seps)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

impl AppConfig {
    /// Load configuration from the process environment.
    pub fn from_env() -> Self {
        // Media roots: prefer MEDIA_ROOTS (list), fall back to MEDIA_ROOT.
        let media_roots = match std::env::var("MEDIA_ROOTS") {
            Ok(v) if !v.trim().is_empty() => split_roots(&v),
            _ => match std::env::var("MEDIA_ROOT") {
                Ok(v) if !v.trim().is_empty() => split_roots(&v),
                _ => Vec::new(),
            },
        };

        let data_dir = PathBuf::from(env_str("DATA_DIR", "data"));
        let db_path = PathBuf::from(env_str(
            "DB_PATH",
            &data_dir.join("transflator.db").to_string_lossy(),
        ));

        let overwrite = env_str("OVERWRITE_BEHAVIOR", "suffix");
        let overwrite_behavior =
            OverwriteBehavior::parse(&overwrite).unwrap_or(OverwriteBehavior::Suffix);

        Self {
            media_roots,
            db_path,
            data_dir: data_dir.clone(),
            app_secret: env_str("APP_SECRET", "dev-insecure-secret-change-me"),
            auth_token: env_str("AUTH_TOKEN", ""),
            host: env_str("HOST", "0.0.0.0"),
            port: env_str("PORT", "8080").parse().unwrap_or(8080),
            llm_subtrans_home: PathBuf::from(env_str("LLM_SUBTRANS_HOME", "/opt/llm-subtrans")),
            python_bin: env_str("PYTHON_BIN", "python3"),
            ffmpeg_bin: env_str("FFMPEG_BIN", "ffmpeg"),
            ffprobe_bin: env_str("FFPROBE_BIN", "ffprobe"),
            concurrency: env_usize("CONCURRENCY", 2),
            output_pattern: env_str("OUTPUT_PATTERN", "{name}.{lang}.srt"),
            overwrite_behavior,
            default_target_language: env_str("DEFAULT_TARGET_LANGUAGE", "English"),
            ffprobe_timeout_secs: env_u64("FFPROBE_TIMEOUT_SECS", 30),
            ffmpeg_timeout_secs: env_u64("FFMPEG_TIMEOUT_SECS", 300),
            web_root: PathBuf::from(env_str("WEB_ROOT", "frontend/dist")),
            log_level: env_str("LOG_LEVEL", "info"),
        }
    }

    /// True when a non-default secret is in use.
    pub fn using_default_secret(&self) -> bool {
        self.app_secret == "dev-insecure-secret-change-me"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_roots_handles_separators_and_whitespace() {
        // `;` is the portable separator (works on Windows and Unix).
        let roots = split_roots("/a/b; /c/d; /e/f;");
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/a/b"),
                PathBuf::from("/c/d"),
                PathBuf::from("/e/f")
            ]
        );
    }

    #[test]
    fn split_roots_keeps_windows_drive_letters() {
        // On Windows, `:` inside a drive letter must not act as a separator.
        #[cfg(windows)]
        {
            let roots = split_roots(r"C:\media; D:\tv");
            assert_eq!(
                roots,
                vec![PathBuf::from(r"C:\media"), PathBuf::from(r"D:\tv")]
            );
        }
        #[cfg(not(windows))]
        {
            let _ = split_roots("/a; /b");
        }
    }

    #[test]
    fn overwrite_behavior_parses() {
        assert_eq!(OverwriteBehavior::parse("overwrite"), Some(OverwriteBehavior::Overwrite));
        assert_eq!(OverwriteBehavior::parse("Suffix"), Some(OverwriteBehavior::Suffix));
        assert_eq!(OverwriteBehavior::parse("skip"), Some(OverwriteBehavior::Skip));
        assert_eq!(OverwriteBehavior::parse("bogus"), None);
    }
}
