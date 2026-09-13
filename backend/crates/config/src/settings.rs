//! Key/value settings store backed by the `settings` table.
//!
//! Secret values (API keys) are encrypted at rest using [`Secrets`]; non-secret
//! values are stored in plaintext. Reads of secrets return the decrypted value.

use crate::app_config::{AppConfig, OverwriteBehavior};
use crate::secrets::Secrets;
use sqlx::Row;
use sqlx::SqlitePool;

/// Setting keys, shared by the API and the jobs crate.
pub mod keys {
    pub const DEFAULT_TARGET_LANGUAGE: &str = "default_target_language";
    /// Last-selected provider (UI preference; pre-fills the translate form).
    pub const DEFAULT_PROVIDER: &str = "default_provider";
    /// Last-selected model (UI preference; pre-fills the translate form).
    pub const DEFAULT_MODEL: &str = "default_model";
    pub const OUTPUT_PATTERN: &str = "output_pattern";
    pub const OVERWRITE_BEHAVIOR: &str = "overwrite_behavior";
    pub const CONCURRENCY: &str = "concurrency";
    /// Secret: the shared access token (NFR-7).
    pub const AUTH_TOKEN: &str = "auth_token";
}

/// The settings key for a provider's API key, e.g. `api_key.openai`.
pub fn api_key_key(provider: &str) -> String {
    format!("api_key.{provider}")
}

/// Effective (stored-overrides-env) values for the settings that affect job
/// execution and auth. Resolved from the `settings` table with [`AppConfig`]
/// (env) as the fallback for any key not set in the UI.
#[derive(Debug, Clone)]
pub struct EffectiveSettings {
    pub default_target_language: String,
    pub output_pattern: String,
    pub overwrite_behavior: OverwriteBehavior,
    pub concurrency: usize,
    pub auth_token: String,
}

/// Reads and writes user-configurable settings.
pub struct SettingsStore<'a> {
    pool: &'a SqlitePool,
    secrets: &'a Secrets,
}

impl<'a> SettingsStore<'a> {
    pub fn new(pool: &'a SqlitePool, secrets: &'a Secrets) -> Self {
        Self { pool, secrets }
    }

    /// Get a non-secret setting value.
    pub async fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(self.pool)
            .await?;
        Ok(row.and_then(|r| r.try_get::<String, _>(0).ok()))
    }

    /// Set a non-secret setting value.
    pub async fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO settings (key, value, is_secret) VALUES (?, ?, 0) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, is_secret = 0")
            .bind(key)
            .bind(value)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Get a secret setting value (decrypted).
    pub async fn get_secret(&self, key: &str) -> anyhow::Result<Option<String>> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = ? AND is_secret = 1")
            .bind(key)
            .fetch_optional(self.pool)
            .await?;
        let Some(row) = row else { return Ok(None) };
        let enc: String = row.try_get(0)?;
        let dec = self.secrets.decrypt(&enc)?;
        Ok(Some(dec))
    }

    /// Set a secret setting value (encrypted at rest).
    pub async fn set_secret(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let enc = self.secrets.encrypt(value)?;
        sqlx::query("INSERT INTO settings (key, value, is_secret) VALUES (?, ?, 1) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, is_secret = 1")
            .bind(key)
            .bind(&enc)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Delete a setting.
    pub async fn delete(&self, key: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM settings WHERE key = ?")
            .bind(key)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Resolve the effective settings: the stored value for each key if set in
    /// the UI, otherwise the env default from [`AppConfig`].
    pub async fn effective(&self, defaults: &AppConfig) -> anyhow::Result<EffectiveSettings> {
        let default_target_language = self
            .get(keys::DEFAULT_TARGET_LANGUAGE)
            .await?
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| defaults.default_target_language.clone());
        let output_pattern = self
            .get(keys::OUTPUT_PATTERN)
            .await?
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| defaults.output_pattern.clone());
        let overwrite_behavior = match self.get(keys::OVERWRITE_BEHAVIOR).await? {
            Some(s) => OverwriteBehavior::parse(&s).unwrap_or(defaults.overwrite_behavior),
            None => defaults.overwrite_behavior,
        };
        let concurrency = match self.get(keys::CONCURRENCY).await? {
            Some(s) => s
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .unwrap_or(defaults.concurrency),
            None => defaults.concurrency,
        };
        let auth_token = self
            .get_secret(keys::AUTH_TOKEN)
            .await?
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| defaults.auth_token.clone());
        Ok(EffectiveSettings {
            default_target_language,
            output_pattern,
            overwrite_behavior,
            concurrency,
            auth_token,
        })
    }

    /// List all setting keys with a flag indicating whether each is a secret.
    /// Secret values are never returned here (only their masked form, if any).
    pub async fn list(
        &self,
    ) -> anyhow::Result<Vec<(String, bool, Option<String>)>> {
        let rows = sqlx::query("SELECT key, is_secret, value FROM settings ORDER BY key")
            .fetch_all(self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let key: String = r.try_get(0)?;
            let is_secret: i64 = r.try_get(1)?;
            let value: String = r.try_get(2)?;
            let masked = if is_secret == 1 {
                self.secrets
                    .decrypt(&value)
                    .ok()
                    .map(|v| Secrets::mask(&v))
            } else {
                Some(value)
            };
            out.push((key, is_secret == 1, masked));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn pool() -> SqlitePool {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.db");
        std::mem::forget(dir);
        db::init_pool(&path).await.unwrap()
    }

    fn config() -> AppConfig {
        AppConfig {
            media_roots: vec![],
            db_path: std::path::PathBuf::from("x.db"),
            data_dir: std::path::PathBuf::from("data"),
            app_secret: "test".into(),
            auth_token: "env-token".into(),
            host: "127.0.0.1".into(),
            port: 8080,
            llm_subtrans_home: std::path::PathBuf::from("/opt/llm-subtrans"),
            python_bin: "python3".into(),
            ffmpeg_bin: "ffmpeg".into(),
            ffprobe_bin: "ffprobe".into(),
            concurrency: 2,
            output_pattern: "{name}.{lang}.srt".into(),
            overwrite_behavior: OverwriteBehavior::Suffix,
            default_target_language: "English".into(),
            ffprobe_timeout_secs: 30,
            ffmpeg_timeout_secs: 300,
            web_root: std::path::PathBuf::from("frontend/dist"),
            log_level: "info".into(),
        }
    }

    #[tokio::test]
    async fn effective_falls_back_to_env_defaults() {
        let p = pool().await;
        let secrets = Secrets::from_app_secret("test");
        let store = SettingsStore::new(&p, &secrets);
        let cfg = config();
        let eff = store.effective(&cfg).await.unwrap();
        assert_eq!(eff.default_target_language, "English");
        assert_eq!(eff.output_pattern, "{name}.{lang}.srt");
        assert_eq!(eff.overwrite_behavior, OverwriteBehavior::Suffix);
        assert_eq!(eff.concurrency, 2);
        assert_eq!(eff.auth_token, "env-token");
    }

    #[tokio::test]
    async fn effective_prefers_stored_values() {
        let p = pool().await;
        let secrets = Secrets::from_app_secret("test");
        let store = SettingsStore::new(&p, &secrets);
        store.set(keys::DEFAULT_TARGET_LANGUAGE, "日本語").await.unwrap();
        store.set(keys::OUTPUT_PATTERN, "{name}_sub.srt").await.unwrap();
        store.set(keys::OVERWRITE_BEHAVIOR, "skip").await.unwrap();
        store.set(keys::CONCURRENCY, "7").await.unwrap();
        store.set_secret(keys::AUTH_TOKEN, "stored-token").await.unwrap();
        let cfg = config();
        let eff = store.effective(&cfg).await.unwrap();
        assert_eq!(eff.default_target_language, "日本語");
        assert_eq!(eff.output_pattern, "{name}_sub.srt");
        assert_eq!(eff.overwrite_behavior, OverwriteBehavior::Skip);
        assert_eq!(eff.concurrency, 7);
        assert_eq!(eff.auth_token, "stored-token");
    }

    #[tokio::test]
    async fn effective_ignores_blank_and_invalid() {
        let p = pool().await;
        let secrets = Secrets::from_app_secret("test");
        let store = SettingsStore::new(&p, &secrets);
        store.set(keys::CONCURRENCY, "0").await.unwrap();
        store.set(keys::OVERWRITE_BEHAVIOR, "bogus").await.unwrap();
        let cfg = config();
        let eff = store.effective(&cfg).await.unwrap();
        assert_eq!(eff.concurrency, 2);
        assert_eq!(eff.overwrite_behavior, OverwriteBehavior::Suffix);
    }
}
