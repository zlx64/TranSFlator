//! Job execution: the `JobRunner` abstraction and the real pipeline
//! (probe → extract → normalize → llm-subtrans → output).

use crate::events::{JobEvent, JobEventEnvelope};
use crate::model::Job;
use crate::store::JobStore;
use futures_util::future::BoxFuture;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use transflator_config::app_config::OverwriteBehavior;
use transflator_config::{AppConfig, Secrets, SettingsStore};
use transflator_media::classify;
use transflator_media::disk::ensure_space;
use transflator_media::encoding::normalize_file_in_place;
use transflator_media::ffmpeg::Ffmpeg;
use transflator_media::ffprobe::Ffprobe;
use transflator_media::model::{Selection, StreamKind, SubtitleKind};
use transflator_media::PathGuard;
use transflator_translate::{
    build_command, classify_failure, kill_tree, parse_progress_line, spawn_translation, validate,
    FailureClass, Provider, TranslateOptions,
};

/// Minimum free space required before extraction (subtitles are small; this
/// catches a full disk early, §6.5).
const MIN_FREE_BYTES: u64 = 64 * 1024 * 1024;

/// Bounded auto-retry for transient translation failures (§6.6). The
/// llm-subtrans project file resumes in place, so a retry continues progress.
const MAX_ATTEMPTS: u32 = 3;

/// Outcome of a job run. `Ok(output_path)` on success.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The job was canceled (FR-17); not an error state.
    #[error("job canceled")]
    Canceled,
    /// The job failed with a user-presentable message.
    #[error("{0}")]
    Failed(String),
}

/// Executes a single job. Abstracted so the queue can be tested with a fake
/// runner (no ffmpeg/python needed).
pub trait JobRunner: Send + Sync {
    fn run<'a>(
        &'a self,
        job: &'a Job,
        cancel: &'a CancellationToken,
        events: &'a broadcast::Sender<JobEventEnvelope>,
    ) -> BoxFuture<'a, Result<String, RunError>>;
}

/// The real pipeline: ffprobe → ffmpeg extraction → UTF-8 normalization →
/// llm-subtrans subprocess.
pub struct PipelineRunner {
    pub config: AppConfig,
    pub secrets: Secrets,
    pub pool: sqlx::sqlite::SqlitePool,
    pub guard: PathGuard,
}

impl PipelineRunner {
    /// The directory where extracted subtitle files live (data dir is RW even
    /// when media roots are read-only).
    pub fn extracted_dir(&self) -> PathBuf {
        self.config.data_dir.join("extracted")
    }

    fn extracted_path(&self, job_id: &str) -> PathBuf {
        self.extracted_dir().join(format!("{job_id}.srt"))
    }

    /// Resolve the output path per the effective `output_pattern` + overwrite
    /// behavior (§6.8). Reads the stored settings (UI values override env).
    async fn resolve_output(&self, source: &Path, target_language: &str) -> Result<PathBuf, RunError> {
        let settings = SettingsStore::new(&self.pool, &self.secrets);
        let eff = settings
            .effective(&self.config)
            .await
            .map_err(|e| RunError::Failed(format!("settings read failed: {e}")))?;
        resolve_output_path(
            &eff.output_pattern,
            eff.overwrite_behavior,
            source,
            target_language,
        )
        .map_err(RunError::Failed)
    }

    /// Resolve the API key: encrypted settings first, then the process env.
    async fn resolve_api_key(&self, provider: Provider) -> Result<Option<String>, RunError> {
        let settings = SettingsStore::new(&self.pool, &self.secrets);
        if let Some(key) = settings
            .get_secret(&format!("api_key.{}", provider.as_str()))
            .await
            .map_err(|e| RunError::Failed(format!("settings read failed: {e}")))?
        {
            if !key.trim().is_empty() {
                return Ok(Some(key));
            }
        }
        if let Some(env_key) = provider.env_key() {
            if let Ok(v) = std::env::var(env_key) {
                if !v.trim().is_empty() {
                    return Ok(Some(v));
                }
            }
        }
        Ok(None)
    }

    /// Best-effort discovery of the llm-subtrans project file written next to
    /// the input file (FR-18). Works for both extracted (`{job_id}.srt`) and
    /// uploaded external subtitles.
    fn find_project_file(&self, input: &Path) -> Option<PathBuf> {
        let dir = input.parent()?;
        let stem = input.file_stem()?.to_string_lossy().to_string();
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&stem) && !name.ends_with(".srt") {
                return Some(entry.path());
            }
        }
        None
    }
}

/// Resolve the output path from the naming pattern + overwrite behavior (§6.8).
///
/// Placeholders: `{name}` (source file stem), `{lang}` (target language),
/// `{ext}` (`srt`). The output lives next to the source file.
pub fn resolve_output_path(
    pattern: &str,
    overwrite: OverwriteBehavior,
    source: &Path,
    target_language: &str,
) -> Result<PathBuf, String> {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "subtitle".into());
    let dir = source.parent().unwrap_or(Path::new("."));
    let name = pattern
        .replace("{name}", &stem)
        .replace("{lang}", target_language)
        .replace("{ext}", "srt");
    let mut candidate = dir.join(&name);

    match overwrite {
        OverwriteBehavior::Overwrite => {}
        OverwriteBehavior::Skip => {
            if candidate.exists() {
                return Err(format!(
                    "output already exists (skip): {}",
                    candidate.display()
                ));
            }
        }
        OverwriteBehavior::Suffix => {
            let ext = candidate
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("srt")
                .to_string();
            let base = candidate.with_extension("").to_string_lossy().to_string();
            for i in 2..=1000 {
                if !candidate.exists() {
                    break;
                }
                candidate = dir.join(format!("{base}-{i}.{ext}"));
            }
        }
    }
    Ok(candidate)
}

impl JobRunner for PipelineRunner {
    fn run<'a>(
        &'a self,
        job: &'a Job,
        cancel: &'a CancellationToken,
        events: &'a broadcast::Sender<JobEventEnvelope>,
    ) -> BoxFuture<'a, Result<String, RunError>> {
        Box::pin(async move {
            let store = JobStore::new(self.pool.clone());
            let emit = |event: JobEvent| {
                let _ = events.send(JobEventEnvelope::new(job.id.clone(), event));
            };

            // 1. Source must still be inside a media root.
            let within_root = self
                .guard
                .roots()
                .iter()
                .any(|root| job.source_path.starts_with(root));
            if !within_root {
                return Err(RunError::Failed(format!(
                    "source is not inside a media root: {}",
                    job.source_path.display()
                )));
            }

            // 2–4. Determine the input. An uploaded external subtitle (§6.1)
            // skips probe/extraction entirely; otherwise probe + extract.
            let input: PathBuf = if let Some(ext) = &job.external_subtitle_path {
                if !ext.exists() {
                    return Err(RunError::Failed(format!(
                        "uploaded subtitle file is missing: {}",
                        ext.display()
                    )));
                }
                let name = ext
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                let msg = format!("using uploaded subtitle: {name}");
                emit(JobEvent::Log { line: msg.clone() });
                store.append_log(&job.id, &msg).await.ok();
                ext.clone()
            } else {
                // 2. Probe the file.
                let probe =
                    Ffprobe::new(&self.config.ffprobe_bin, self.config.ffprobe_timeout_secs);
                let info = probe
                    .probe(&job.source_path)
                    .await
                    .map_err(|e| RunError::Failed(format!("probe failed: {e}")))?;

                // 3. Pick the subtitle stream.
                let stream_index = match job.subtitle_stream_index {
                    Some(idx) => {
                        let stream = info
                            .streams
                            .iter()
                            .find(|s| s.index == idx)
                            .ok_or_else(|| {
                                RunError::Failed(format!(
                                    "subtitle stream #{idx} not found in file"
                                ))
                            })?;
                        if stream.kind != StreamKind::Subtitle {
                            return Err(RunError::Failed(format!(
                                "stream #{idx} is not a subtitle stream"
                            )));
                        }
                        if stream.subtitle_kind == Some(SubtitleKind::Image) {
                            return Err(RunError::Failed(format!(
                                "stream #{idx} is image-based (OCR required) — pick a text track"
                            )));
                        }
                        idx
                    }
                    None => match classify::select_subtitle(&info.streams) {
                        Selection::Auto { stream_index } => stream_index,
                        Selection::Picker => {
                            return Err(RunError::Failed(
                                "file has multiple subtitle tracks — select one".into(),
                            ))
                        }
                        Selection::ImageOnly => {
                            return Err(RunError::Failed(
                                "only image-based subtitle tracks (OCR required)".into(),
                            ))
                        }
                        Selection::None => {
                            return Err(RunError::Failed(
                                "no subtitle tracks in file".into(),
                            ))
                        }
                    },
                };
                emit(JobEvent::Log {
                    line: format!("extracting subtitle stream #{stream_index}"),
                });
                store
                    .append_log(&job.id, &format!("extracting subtitle stream #{stream_index}"))
                    .await
                    .ok();

                // 4. Extract to the data dir (media roots may be read-only).
                let extracted = self.extracted_path(&job.id);
                tokio::fs::create_dir_all(&self.extracted_dir())
                    .await
                    .map_err(|e| RunError::Failed(format!("cannot create extracted dir: {e}")))?;
                ensure_space(&self.extracted_dir(), MIN_FREE_BYTES)
                    .map_err(|e| RunError::Failed(format!("disk space check failed: {e}")))?;
                let ffmpeg =
                    Ffmpeg::new(&self.config.ffmpeg_bin, self.config.ffmpeg_timeout_secs);
                let streams: Vec<transflator_media::model::Stream> = info.streams;
                let stream = streams
                    .iter()
                    .find(|s| s.index == stream_index)
                    .expect("stream index verified above");
                ffmpeg.extract_subtitle(
                    &job.source_path,
                    &streams,
                    stream.index,
                    &extracted,
                )
                .await
                .map_err(|e| RunError::Failed(format!("extraction failed: {e}")))?
            };

            // 5. Normalize to UTF-8 in place.
            match normalize_file_in_place(&input) {
                Ok(n) if !n.was_utf8 => {
                    let msg = format!("normalized encoding to UTF-8");
                    emit(JobEvent::Log { line: msg.clone() });
                    store.append_log(&job.id, &msg).await.ok();
                }
                Ok(_) => {}
                Err(e) => {
                    return Err(RunError::Failed(format!("encoding normalization failed: {e}")))
                }
            }

            // 6. Resolve output path next to the source file.
            let output = self
                .resolve_output(&job.source_path, &job.target_language)
                .await?;

            // 7. Build the llm-subtrans command (fail fast on missing key).
            let provider = Provider::parse(&job.provider).ok_or_else(|| {
                RunError::Failed(format!("unknown provider: {}", job.provider))
            })?;
            let api_key = self.resolve_api_key(provider).await?;
            let mut opts = TranslateOptions::default();
            opts.target_language = job.target_language.clone();
            opts.model = job.model.clone();
            opts.api_key = api_key;
            opts.output = Some(output.clone());
            opts.project = true;
            // Retry mode mapping (FR-18).
            match job.retry_mode.as_str() {
                "retranslate" => opts.retranslate = true,
                "reparse" => opts.reparse = true,
                _ => {}
            }
            validate(provider, &opts).map_err(|e| RunError::Failed(e.to_string()))?;
            let spec = build_command(provider, &opts, &input);

            // 8. Spawn and stream output, retrying transient failures (§6.6).
            'attempt: for attempt in 1..=MAX_ATTEMPTS {
                let start_msg = if attempt == 1 {
                    format!(
                        "starting {} (target: {})",
                        provider.as_str(), job.target_language
                    )
                } else {
                    format!(
                        "retrying {} (attempt {attempt}/{MAX_ATTEMPTS})",
                        provider.as_str()
                    )
                };
                emit(JobEvent::Log { line: start_msg.clone() });
                store.append_log(&job.id, &start_msg).await.ok();

                let mut child = spawn_translation(
                    &self.config.llm_subtrans_home,
                    &self.config.python_bin,
                    &spec,
                )
                .await
                .map_err(|e| RunError::Failed(format!("failed to start llm-subtrans: {e}")))?;

                let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
                let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
                let mut last_pct: i32 = -1;
                let mut stderr_tail: Vec<String> = Vec::new();

                let status = loop {
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            kill_tree(&mut child);
                            return Err(RunError::Canceled);
                        }
                        res = child.wait() => {
                            break res.map_err(|e| RunError::Failed(format!("process error: {e}")))?;
                        }
                        line = stdout.next_line() => {
                            match line {
                                Ok(Some(l)) => {
                                    emit(JobEvent::Log { line: l.clone() });
                                    store.append_log(&job.id, &l).await.ok();
                                    if let Some(p) = parse_progress_line(&l) {
                                        let pct = p.pct as i32;
                                        if pct != last_pct {
                                            last_pct = pct;
                                            emit(JobEvent::Progress { pct });
                                            store.set_progress(&job.id, pct, None).await.ok();
                                        }
                                    }
                                }
                                Ok(None) => {}
                                Err(_) => {}
                            }
                        }
                        line = stderr.next_line() => {
                            if let Ok(Some(l)) = line {
                                emit(JobEvent::Log { line: l.clone() });
                                store.append_log(&job.id, &l).await.ok();
                                stderr_tail.push(l);
                                if stderr_tail.len() > 20 {
                                    stderr_tail.remove(0);
                                }
                            }
                        }
                    }
                };

                if status.success() {
                    break 'attempt;
                }

                let code = status.code().unwrap_or(-1);
                let tail = stderr_tail.join("\n");
                let class = classify_failure(code, &tail);
                if class == FailureClass::Transient && attempt < MAX_ATTEMPTS {
                    let backoff = std::time::Duration::from_secs(2u64.pow(attempt));
                    let wait_msg =
                        format!("transient error — retrying in {}s", backoff.as_secs());
                    emit(JobEvent::Log { line: wait_msg.clone() });
                    store.append_log(&job.id, &wait_msg).await.ok();
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            return Err(RunError::Canceled);
                        }
                        _ = tokio::time::sleep(backoff) => {}
                    }
                    continue 'attempt;
                }

                let msg = if tail.is_empty() {
                    format!("{}: llm-subtrans exited with code {code}", class.label())
                } else {
                    format!(
                        "{}: llm-subtrans exited with code {code}: {tail}",
                        class.label()
                    )
                };
                store.set_error(&job.id, &msg).await.ok();
                return Err(RunError::Failed(msg));
            }

            // 9. Success: verify the output, record the project file.
            if !output.exists() {
                let msg = format!("llm-subtrans finished but output is missing: {}", output.display());
                store.set_error(&job.id, &msg).await.ok();
                return Err(RunError::Failed(msg));
            }
            if let Some(project) = self.find_project_file(&input) {
                store
                    .set_project(&job.id, &project.to_string_lossy())
                    .await
                    .ok();
            }
            Ok(output.to_string_lossy().to_string())
        })
    }
}

/// A test double for the queue: configurable success/failure/cancel behavior.
#[derive(Debug, Clone)]
pub struct FakeRunner {
    /// How long each run takes before it finishes.
    pub delay: std::time::Duration,
    /// If set, every run fails (FR-16 failure path).
    pub fail: bool,
    /// If set, runs hang until canceled (cancellation path).
    pub hang: bool,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self {
            delay: std::time::Duration::from_millis(1),
            fail: false,
            hang: false,
        }
    }
}

impl Default for FakeRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRunner for FakeRunner {
    fn run<'a>(
        &'a self,
        job: &'a Job,
        cancel: &'a CancellationToken,
        events: &'a broadcast::Sender<JobEventEnvelope>,
    ) -> BoxFuture<'a, Result<String, RunError>> {
        let fail = self.fail;
        let hang = self.hang;
        let delay = self.delay;
        Box::pin(async move {
            let _ = events.send(JobEventEnvelope::new(
                job.id.clone(),
                JobEvent::Log { line: "fake: started".into() },
            ));
            if hang {
                cancel.cancelled().await;
                return Err(RunError::Canceled);
            }
            tokio::time::sleep(delay).await;
            let _ = events.send(JobEventEnvelope::new(
                job.id.clone(),
                JobEvent::Progress { pct: 100 },
            ));
            if fail {
                return Err(RunError::Failed("fake: forced failure".into()));
            }
            Ok(format!("fake-output-{}.srt", job.id))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_placeholders_substituted() {
        let out = resolve_output_path(
            "{name}.{lang}.srt",
            OverwriteBehavior::Overwrite,
            Path::new("C:\\media\\Show\\E01.mkv"),
            "de",
        )
        .unwrap();
        assert_eq!(out, Path::new("C:\\media\\Show\\E01.de.srt"));
    }

    #[test]
    fn overwrite_returns_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("E01.mkv");
        std::fs::write(&source, b"x").unwrap();
        let expected = dir.path().join("E01.de.srt");
        std::fs::write(&expected, b"old").unwrap();
        let out = resolve_output_path(
            "{name}.{lang}.srt",
            OverwriteBehavior::Overwrite,
            &source,
            "de",
        )
        .unwrap();
        assert_eq!(out, expected);
    }

    #[test]
    fn suffix_finds_free_name() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("E01.mkv");
        std::fs::write(&source, b"x").unwrap();
        std::fs::write(dir.path().join("E01.de.srt"), b"old").unwrap();
        std::fs::write(dir.path().join("E01.de-2.srt"), b"old2").unwrap();
        let out = resolve_output_path(
            "{name}.{lang}.srt",
            OverwriteBehavior::Suffix,
            &source,
            "de",
        )
        .unwrap();
        assert_eq!(out, dir.path().join("E01.de-3.srt"));
    }

    #[test]
    fn skip_errors_on_existing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("E01.mkv");
        std::fs::write(&source, b"x").unwrap();
        std::fs::write(dir.path().join("E01.de.srt"), b"old").unwrap();
        let err = resolve_output_path(
            "{name}.{lang}.srt",
            OverwriteBehavior::Skip,
            &source,
            "de",
        )
        .unwrap_err();
        assert!(err.contains("skip"), "{err}");
    }

    #[test]
    fn skip_ok_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("E01.mkv");
        std::fs::write(&source, b"x").unwrap();
        let out = resolve_output_path(
            "{name}.{lang}.srt",
            OverwriteBehavior::Skip,
            &source,
            "de",
        )
        .unwrap();
        assert_eq!(out, dir.path().join("E01.de.srt"));
    }
}
