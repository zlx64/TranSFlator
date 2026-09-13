//! Configuration, secrets, and database setup for TranSFlator.
//!
//! This crate owns:
//! - [`AppConfig`]: process configuration loaded from environment variables.
//! - [`secrets`]: AES-256-GCM encryption for API keys at rest.
//! - [`db`]: SQLite pool creation and schema migrations.
//! - [`settings`]: a small key/value store for user-configurable settings,
//!   with transparent encryption of secret values.

pub mod app_config;
pub mod boot;
pub mod db;
pub mod secrets;
pub mod settings;

pub use app_config::AppConfig;
pub use secrets::Secrets;
pub use settings::{EffectiveSettings, SettingsStore};
