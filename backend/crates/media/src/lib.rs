//! ffprobe/ffmpeg wrappers, stream classification, path safety, and the media
//! cache for TranSFlator.
//!
//! All external-tool interaction is through subprocesses with explicit timeouts
//! (spec §8.1, NFR-5). No whole video file is ever loaded into memory.

pub mod cache;
pub mod classify;
pub mod disk;
pub mod encoding;
pub mod ffprobe;
pub mod ffmpeg;
pub mod library;
pub mod model;
pub mod path_guard;

#[cfg(test)]
mod stubs;

pub use model::{MediaError, MediaInfo, Selection, Stream, StreamKind, SubtitleKind};
pub use path_guard::PathGuard;
