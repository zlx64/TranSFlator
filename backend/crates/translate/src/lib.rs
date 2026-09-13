//! llm-subtrans integration: CLI flag mapping, progress parsing, and process
//! spawning.
//!
//! Translation is delegated to the existing [llm-subtrans](https://github.com/machinewrapped/llm-subtrans)
//! Python tool — we never reimplement translation logic (spec §8.2). This crate
//! maps job settings to the exact command line, parses progress from the tool's
//! output, and spawns/kills the subprocess.

pub mod command;
pub mod failure;
pub mod models;
pub mod progress;
pub mod provider;
pub mod runner;

pub use command::{build_command, validate, CommandSpec, TranslateError, TranslateOptions};
pub use failure::{classify_failure, FailureClass};
pub use models::{custom_models_url, list_models, list_models_custom, ModelsError};
pub use progress::{parse_progress_line, Progress};
pub use provider::Provider;
pub use runner::{kill_tree, spawn_translation};
