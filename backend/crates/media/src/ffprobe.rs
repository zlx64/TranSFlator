//! `ffprobe` subprocess wrapper (spec §8.1).
//!
//! Runs `ffprobe -print_format json -show_streams -show_format` with an explicit
//! timeout, capturing stdout and stderr. Non-zero exit or timeout produce a
//! specific, actionable error (§6.3) rather than a generic failure.

use crate::classify::classify_subtitle_codec;
use crate::model::{MediaError, MediaInfo, Stream, StreamKind};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

/// A configured ffprobe runner.
#[derive(Debug, Clone)]
pub struct Ffprobe {
    bin: String,
    timeout: Duration,
}

#[derive(Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfStream>,
    #[serde(default)]
    format: Option<FfFormat>,
}

#[derive(Deserialize)]
struct FfStream {
    index: usize,
    codec_type: String,
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    tags: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    disposition: Option<FfDisposition>,
}

#[derive(Deserialize)]
struct FfDisposition {
    #[serde(default)]
    default: i64,
    #[serde(default)]
    forced: i64,
}

#[derive(Deserialize)]
struct FfFormat {
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    format_name: Option<String>,
}

impl Ffprobe {
    pub fn new(bin: &str, timeout_secs: u64) -> Self {
        Self {
            bin: bin.to_string(),
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    /// Probe a media file and return parsed stream information.
    pub async fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format"])
            .arg(path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output = match tokio::time::timeout(self.timeout, cmd.output()).await {
            Ok(result) => result
                .map_err(|e| MediaError::Ffprobe(format!("failed to spawn {bin}: {e}", bin = self.bin)))?,
            Err(_) => return Err(MediaError::FfprobeTimeout(self.timeout.as_secs())),
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output.status.code().unwrap_or(-1);
            return Err(MediaError::Ffprobe(format!(
                "ffprobe exited {code}: {stderr} — the file may be corrupted or use an unsupported container"
            )));
        }

        let parsed: FfprobeOutput =
            serde_json::from_slice(&output.stdout).map_err(MediaError::Json)?;
        Ok(self.to_media_info(path, &parsed))
    }

    fn to_media_info(&self, path: &Path, out: &FfprobeOutput) -> MediaInfo {
        let mut streams = Vec::with_capacity(out.streams.len());
        let mut video_count = 0usize;
        let mut audio_count = 0usize;
        let mut subtitle_count = 0usize;

        for s in &out.streams {
            let kind = match s.codec_type.as_str() {
                "video" => {
                    video_count += 1;
                    StreamKind::Video
                }
                "audio" => {
                    audio_count += 1;
                    StreamKind::Audio
                }
                "subtitle" => {
                    subtitle_count += 1;
                    StreamKind::Subtitle
                }
                _ => StreamKind::Unknown,
            };

            let codec = s.codec_name.clone().unwrap_or_default();
            let tags = s.tags.as_ref();
            let language = tags
                .and_then(|t| t.get("language"))
                .cloned()
                .filter(|l| !l.is_empty() && l != "und");
            let title = tags.and_then(|t| t.get("title")).cloned();
            let default = s.disposition.as_ref().map(|d| d.default != 0).unwrap_or(false);
            let forced = s.disposition.as_ref().map(|d| d.forced != 0).unwrap_or(false);
            let subtitle_kind = if kind == StreamKind::Subtitle {
                classify_subtitle_codec(&codec)
            } else {
                None
            };

            streams.push(Stream {
                index: s.index,
                kind,
                codec,
                language,
                title,
                default,
                forced,
                subtitle_kind,
            });
        }

        let format_name = out.format.as_ref().and_then(|f| f.format_name.clone());
        let container = format_name
            .as_deref()
            .map(|f| f.split(',').next().unwrap_or(f).trim().to_string());

        MediaInfo {
            path: path.to_string_lossy().to_string(),
            duration_s: out
                .format
                .as_ref()
                .and_then(|f| f.duration.as_deref())
                .and_then(|d| d.parse().ok()),
            container,
            format_name,
            size: out
                .format
                .as_ref()
                .and_then(|f| f.size.as_deref())
                .and_then(|s| s.parse().ok()),
            streams,
            video_count,
            audio_count,
            subtitle_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a canned ffprobe JSON document (no subprocess needed).
    fn parse(json: &str) -> MediaInfo {
        let out: FfprobeOutput = serde_json::from_str(json).unwrap();
        Ffprobe::new("ffprobe", 30).to_media_info(Path::new("/x.mkv"), &out)
    }

    #[test]
    fn parses_streams_and_counts() {
        let json = r#"{
          "streams": [
            {"index":0,"codec_type":"video","codec_name":"h264"},
            {"index":1,"codec_type":"audio","codec_name":"aac","tags":{"language":"eng"}},
            {"index":2,"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"jpn","title":"Main"},"disposition":{"default":1,"forced":0}},
            {"index":3,"codec_type":"subtitle","codec_name":"hdmv_pgs_subtitle","tags":{"language":"eng"},"disposition":{"default":0,"forced":1}}
          ],
          "format": {"format_name":"matroska,webm","duration":"3600.5","size":"123456"}
        }"#;
        let info = parse(json);
        assert_eq!(info.video_count, 1);
        assert_eq!(info.audio_count, 1);
        assert_eq!(info.subtitle_count, 2);
        assert_eq!(info.duration_s, Some(3600.5));
        assert_eq!(info.container.as_deref(), Some("matroska"));
        assert_eq!(info.size, Some(123456));

        let s2 = &info.streams[2];
        assert_eq!(s2.kind, StreamKind::Subtitle);
        assert_eq!(s2.language.as_deref(), Some("jpn"));
        assert!(s2.default);
        assert!(!s2.forced);
        assert_eq!(
            s2.subtitle_kind,
            Some(crate::model::SubtitleKind::Text)
        );

        let s3 = &info.streams[3];
        assert!(s3.forced);
        assert_eq!(
            s3.subtitle_kind,
            Some(crate::model::SubtitleKind::Image)
        );
    }

    #[test]
    fn missing_fields_default_safely() {
        let json = r#"{ "streams": [ {"index":0,"codec_type":"video"} ], "format": {} }"#;
        let info = parse(json);
        assert_eq!(info.streams.len(), 1);
        assert_eq!(info.streams[0].codec, "");
        assert_eq!(info.duration_s, None);
    }

    // ---- Subprocess tests (§6.3): specific, actionable errors ----

    #[tokio::test]
    async fn probe_nonzero_exit_reports_code_and_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let bin = crate::stubs::failing_tool(
            dir.path(),
            "ffprobe",
            "No such file or directory",
            1,
        );
        let input = dir.path().join("x.mkv");
        std::fs::write(&input, b"x").unwrap();
        let probe = Ffprobe::new(bin.to_str().unwrap(), 10);
        let err = probe.probe(&input).await.unwrap_err();
        match err {
            MediaError::Ffprobe(msg) => {
                assert!(msg.contains("exited 1"), "{msg}");
                assert!(msg.contains("No such file or directory"), "{msg}");
                assert!(msg.contains("corrupted"), "{msg}");
            }
            other => panic!("expected Ffprobe error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn probe_timeout_is_specific() {
        let dir = tempfile::tempdir().unwrap();
        let bin = crate::stubs::sleeper_tool(dir.path(), "ffprobe");
        let input = dir.path().join("x.mkv");
        std::fs::write(&input, b"x").unwrap();
        let probe = Ffprobe::new(bin.to_str().unwrap(), 1);
        let err = probe.probe(&input).await.unwrap_err();
        assert!(matches!(err, MediaError::FfprobeTimeout(1)), "{err:?}");
    }
}
