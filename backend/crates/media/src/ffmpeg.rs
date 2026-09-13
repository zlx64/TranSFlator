//! `ffmpeg` subtitle-extraction wrapper (spec §8.1, FR-8).
//!
//! Only the subtitle stream is ever touched — video/audio are never re-encoded.
//! On any failure (non-zero exit, timeout, empty output) the partial output
//! file is removed so no zero-byte artifact is left behind (§6.4).

use crate::model::{MediaError, Stream, StreamKind};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

/// A configured ffmpeg runner.
#[derive(Debug, Clone)]
pub struct Ffmpeg {
    bin: String,
    timeout: Duration,
}

impl Ffmpeg {
    pub fn new(bin: &str, timeout_secs: u64) -> Self {
        Self {
            bin: bin.to_string(),
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    /// Extract the subtitle stream with the given **overall** ffprobe index to
    /// `output` as UTF-8-agnostic SRT (encoding normalization happens later).
    ///
    /// `streams` is the full ffprobe stream list; the overall index is mapped to
    /// the subtitle-only ordinal that `ffmpeg -map 0:s:N` expects.
    pub async fn extract_subtitle(
        &self,
        input: &Path,
        streams: &[Stream],
        overall_index: usize,
        output: &Path,
    ) -> Result<PathBuf, MediaError> {
        let ordinal = streams
            .iter()
            .filter(|s| s.kind == StreamKind::Subtitle)
            .position(|s| s.index == overall_index)
            .ok_or_else(|| {
                MediaError::Ffmpeg(format!(
                    "stream index {overall_index} is not a subtitle stream"
                ))
            })?;

        if let Some(parent) = output.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        // Start from a clean slate.
        let _ = std::fs::remove_file(output);

        let map = format!("0:s:{ordinal}");
        let mut cmd = Command::new(&self.bin);
        cmd.args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            input.to_str().ok_or_else(|| MediaError::Ffmpeg("input path not valid UTF-8".into()))?,
            "-map",
            &map,
            "-c:s",
            "srt",
        ])
        .arg(output)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

        let result = tokio::time::timeout(self.timeout, cmd.output()).await;

        match result {
            Err(_) => {
                remove_quiet(output);
                Err(MediaError::FfmpegTimeout(self.timeout.as_secs()))
            }
            Ok(Err(e)) => {
                remove_quiet(output);
                Err(MediaError::Ffmpeg(format!(
                    "failed to spawn {bin}: {e}",
                    bin = self.bin
                )))
            }
            Ok(Ok(out)) => {
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let code = out.status.code().unwrap_or(-1);
                    remove_quiet(output);
                    return Err(MediaError::Ffmpeg(format!(
                        "ffmpeg exited {code}: {stderr}"
                    )));
                }
                // Guard against a zero-byte / partial output file (§6.4).
                match std::fs::metadata(output) {
                    Ok(m) if m.len() > 0 => Ok(output.to_path_buf()),
                    _ => {
                        remove_quiet(output);
                        Err(MediaError::Ffmpeg(
                            "ffmpeg succeeded but produced an empty output file".into(),
                        ))
                    }
                }
            }
        }
    }
}

fn remove_quiet(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::classify_subtitle_codec;

    fn streams() -> Vec<Stream> {
        vec![
            Stream {
                index: 0,
                kind: StreamKind::Video,
                codec: "h264".into(),
                language: None,
                title: None,
                default: false,
                forced: false,
                subtitle_kind: None,
            },
            Stream {
                index: 1,
                kind: StreamKind::Audio,
                codec: "aac".into(),
                language: None,
                title: None,
                default: true,
                forced: false,
                subtitle_kind: None,
            },
            Stream {
                index: 2,
                kind: StreamKind::Subtitle,
                codec: "subrip".into(),
                language: Some("jpn".into()),
                title: None,
                default: true,
                forced: false,
                subtitle_kind: classify_subtitle_codec("subrip"),
            },
            Stream {
                index: 3,
                kind: StreamKind::Subtitle,
                codec: "ass".into(),
                language: Some("eng".into()),
                title: Some("Signs".into()),
                default: false,
                forced: true,
                subtitle_kind: classify_subtitle_codec("ass"),
            },
        ]
    }

    /// Compute the ordinal the way `extract_subtitle` does (shared logic check).
    fn ordinal(streams: &[Stream], overall: usize) -> Option<usize> {
        streams
            .iter()
            .filter(|s| s.kind == StreamKind::Subtitle)
            .position(|s| s.index == overall)
    }

    #[test]
    fn maps_overall_index_to_subtitle_ordinal() {
        let s = streams();
        // Overall index 2 is the first subtitle -> ordinal 0.
        assert_eq!(ordinal(&s, 2), Some(0));
        // Overall index 3 is the second subtitle -> ordinal 1.
        assert_eq!(ordinal(&s, 3), Some(1));
        // Overall index 0 (video) is not a subtitle.
        assert_eq!(ordinal(&s, 0), None);
        // Unknown index.
        assert_eq!(ordinal(&s, 99), None);
    }

    // ---- Subprocess tests (§6.4): failure captures stderr, no partial file ----

    const SRT: &str = "1\n00:00:00,000 --> 00:00:02,000\nHello";

    #[tokio::test]
    async fn extract_failure_reports_stderr_and_removes_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        // The stub writes a partial file, then fails.
        let bin =
            crate::stubs::writer_tool(dir.path(), "ffmpeg", "partial", "Conversion failed!", 1);
        let input = dir.path().join("x.mkv");
        std::fs::write(&input, b"x").unwrap();
        let output = dir.path().join("out.srt");
        let ffmpeg = Ffmpeg::new(bin.to_str().unwrap(), 10);
        let err = ffmpeg
            .extract_subtitle(&input, &streams(), 2, &output)
            .await
            .unwrap_err();
        match err {
            MediaError::Ffmpeg(msg) => {
                assert!(msg.contains("exited 1"), "{msg}");
                assert!(msg.contains("Conversion failed!"), "{msg}");
            }
            other => panic!("expected Ffmpeg error, got {other:?}"),
        }
        assert!(!output.exists(), "partial output file must be removed");
    }

    #[tokio::test]
    async fn extract_timeout_removes_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let bin = crate::stubs::sleeper_tool(dir.path(), "ffmpeg");
        let input = dir.path().join("x.mkv");
        std::fs::write(&input, b"x").unwrap();
        let output = dir.path().join("out.srt");
        let ffmpeg = Ffmpeg::new(bin.to_str().unwrap(), 1);
        let err = ffmpeg
            .extract_subtitle(&input, &streams(), 2, &output)
            .await
            .unwrap_err();
        assert!(matches!(err, MediaError::FfmpegTimeout(1)), "{err:?}");
        assert!(!output.exists(), "partial output file must be removed");
    }

    #[tokio::test]
    async fn extract_empty_output_rejected_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let bin = crate::stubs::failing_tool(dir.path(), "ffmpeg", "", 0);
        let input = dir.path().join("x.mkv");
        std::fs::write(&input, b"x").unwrap();
        let output = dir.path().join("out.srt");
        let ffmpeg = Ffmpeg::new(bin.to_str().unwrap(), 10);
        let err = ffmpeg
            .extract_subtitle(&input, &streams(), 2, &output)
            .await
            .unwrap_err();
        match err {
            MediaError::Ffmpeg(msg) => assert!(msg.contains("empty output file"), "{msg}"),
            other => panic!("expected Ffmpeg error, got {other:?}"),
        }
        assert!(!output.exists());
    }

    #[tokio::test]
    async fn extract_success_writes_output() {
        let dir = tempfile::tempdir().unwrap();
        let bin = crate::stubs::writer_tool(dir.path(), "ffmpeg", SRT, "", 0);
        let input = dir.path().join("x.mkv");
        std::fs::write(&input, b"x").unwrap();
        let output = dir.path().join("out.srt");
        let ffmpeg = Ffmpeg::new(bin.to_str().unwrap(), 10);
        let out = ffmpeg
            .extract_subtitle(&input, &streams(), 2, &output)
            .await
            .unwrap();
        assert_eq!(out, output);
        let content = std::fs::read_to_string(&output).unwrap();
        assert!(content.contains("Hello"), "{content}");
        assert!(content.contains("-->"), "{content}");
    }
}
