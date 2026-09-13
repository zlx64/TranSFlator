//! SQLite persistence for jobs (spec §9 `jobs` table).

use crate::machine;
use crate::model::{Job, JobStatus, NewJob};
use chrono::Utc;
use sqlx::sqlite::SqlitePool;
use sqlx::{FromRow, Row};
use std::path::PathBuf;
use thiserror::Error;

/// Maximum bytes of log retained in `log_tail` (FR-16).
pub const LOG_TAIL_MAX: usize = 16 * 1024;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid job transition {0}")]
    Transition(#[from] crate::machine::TransitionError),
    #[error("unknown job status: {0}")]
    BadStatus(String),
    #[error("job not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, FromRow)]
struct JobRow {
    id: String,
    source_path: String,
    subtitle_stream_index: Option<i64>,
    source_language: Option<String>,
    target_language: String,
    provider: String,
    model: Option<String>,
    status: String,
    progress_pct: i32,
    output_path: Option<String>,
    project_file_path: Option<String>,
    external_subtitle_path: Option<String>,
    log_tail: String,
    error_message: Option<String>,
    #[sqlx(default)]
    retry_mode: String,
    created_at: String,
    updated_at: String,
}

impl JobRow {
    fn into_job(self) -> Result<Job> {
        Ok(Job {
            id: self.id,
            source_path: PathBuf::from(&self.source_path),
            subtitle_stream_index: self.subtitle_stream_index.map(|v| v as usize),
            source_language: self.source_language,
            target_language: self.target_language,
            provider: self.provider,
            model: self.model,
            status: JobStatus::parse(&self.status).ok_or(StoreError::BadStatus(self.status))?,
            progress_pct: self.progress_pct,
            output_path: self.output_path.as_deref().map(PathBuf::from),
            project_file_path: self.project_file_path.as_deref().map(PathBuf::from),
            external_subtitle_path: self
                .external_subtitle_path
                .as_deref()
                .map(PathBuf::from),
            log_tail: self.log_tail,
            error_message: self.error_message,
            retry_mode: self.retry_mode,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, Clone)]
pub struct JobStore {
    pool: SqlitePool,
}

impl JobStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert a new job in `queued` state and return it.
    pub async fn create(&self, new: &NewJob) -> Result<Job> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO jobs (
                id, source_path, subtitle_stream_index, source_language,
                target_language, provider, model, status, progress_pct,
                external_subtitle_path, log_tail, retry_mode, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, 'queued', 0, ?, '', 'rerun', ?, ?)"#,
        )
        .bind(&id)
        .bind(new.source_path.to_string_lossy().to_string())
        .bind(new.subtitle_stream_index.map(|v| v as i64))
        .bind(&new.source_language)
        .bind(&new.target_language)
        .bind(&new.provider)
        .bind(&new.model)
        .bind(new.external_subtitle_path.as_ref().map(|p| p.to_string_lossy().to_string()))
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &str) -> Result<Job> {
        let row: Option<JobRow> = sqlx::query_as("SELECT * FROM jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| r.into_job()).transpose()?.ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    /// List jobs, most recent first.
    pub async fn list(&self, limit: i64) -> Result<Vec<Job>> {
        let rows: Vec<JobRow> =
            sqlx::query_as("SELECT * FROM jobs ORDER BY created_at DESC, id DESC LIMIT ?")
                .bind(limit)
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter().map(|r| r.into_job()).collect()
    }

    /// Set the job status, enforcing the state machine.
    pub async fn set_status(&self, id: &str, to: JobStatus) -> Result<()> {
        let from = self.get(id).await?.status;
        machine::transition(from, to)?;
        self.update_status_raw(id, to).await
    }

    async fn update_status_raw(&self, id: &str, status: JobStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE jobs SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Set progress (0–100) and, when the job finishes, the output path.
    pub async fn set_progress(&self, id: &str, pct: i32, output_path: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        if let Some(out) = output_path {
            sqlx::query("UPDATE jobs SET progress_pct = ?, output_path = ?, updated_at = ? WHERE id = ?")
                .bind(pct.clamp(0, 100))
                .bind(out)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        } else {
            sqlx::query("UPDATE jobs SET progress_pct = ?, updated_at = ? WHERE id = ?")
                .bind(pct.clamp(0, 100))
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// Append a line to the log, retaining only the last `LOG_TAIL_MAX` bytes.
    pub async fn append_log(&self, id: &str, line: &str) -> Result<()> {
        let job = self.get(id).await?;
        let mut tail = job.log_tail;
        tail.push_str(line);
        tail.push('\n');
        if tail.len() > LOG_TAIL_MAX {
            let excess = tail.len() - LOG_TAIL_MAX;
            // Truncate on a char boundary: slicing at a raw byte offset panics
            // when it lands inside a multi-byte UTF-8 char (e.g. Cyrillic).
            let start = tail.floor_char_boundary(excess);
            tail = tail[start..].to_string();
        }
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE jobs SET log_tail = ?, updated_at = ? WHERE id = ?")
            .bind(&tail)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Record the llm-subtrans project file path (FR-18).
    pub async fn set_project(&self, id: &str, path: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE jobs SET project_file_path = ?, updated_at = ? WHERE id = ?")
            .bind(path)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Record the failure message.
    pub async fn set_error(&self, id: &str, message: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE jobs SET error_message = ?, updated_at = ? WHERE id = ?")
            .bind(message)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Prepare a retry (FR-18): set the retry mode, reset progress/error, and
    /// move the job back to `queued` (state machine enforced by caller).
    pub async fn prepare_retry(&self, id: &str, mode: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE jobs SET status = 'queued', retry_mode = ?, progress_pct = 0, \
             error_message = NULL, log_tail = '', updated_at = ? WHERE id = ?",
        )
        .bind(mode)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// On boot: mark any `running` jobs as `interrupted` (§6.11).
    pub async fn recover_interrupted(&self) -> Result<Vec<String>> {
        let rows: Vec<sqlx::sqlite::SqliteRow> =
            sqlx::query("SELECT id FROM jobs WHERE status = 'running'")
                .fetch_all(&self.pool)
                .await?;
        let ids: Vec<String> = rows.iter().map(|r| r.try_get::<String, _>(0).unwrap_or_default()).collect();
        for id in &ids {
            self.update_status_raw(id, JobStatus::Interrupted).await?;
        }
        Ok(ids)
    }

    /// On boot: re-queue jobs that were `queued` when the process died.
    pub async fn pending_queued(&self) -> Result<Vec<Job>> {
        let rows: Vec<JobRow> =
            sqlx::query_as("SELECT * FROM jobs WHERE status = 'queued' ORDER BY created_at")
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter().map(|r| r.into_job()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transflator_config::db;

    async fn pool() -> SqlitePool {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.db");
        std::mem::forget(dir);
        db::init_pool(&path).await.unwrap()
    }

    fn new_job() -> NewJob {
        NewJob {
            source_path: std::path::PathBuf::from("C:\\media\\E01.mkv"),
            subtitle_stream_index: Some(2),
            source_language: Some("en".into()),
            target_language: "de".into(),
            provider: "openai".into(),
            model: Some("gpt-4o-mini".into()),
            external_subtitle_path: None,
        }
    }

    #[tokio::test]
    async fn create_and_get_roundtrip() {
        let store = JobStore::new(pool().await);
        let job = store.create(&new_job()).await.unwrap();
        assert_eq!(job.status, JobStatus::Queued);
        assert_eq!(job.progress_pct, 0);
        assert_eq!(job.subtitle_stream_index, Some(2));

        let got = store.get(&job.id).await.unwrap();
        assert_eq!(got.id, job.id);
        assert_eq!(got.source_path, job.source_path);
        assert_eq!(got.provider, "openai");
    }

    #[tokio::test]
    async fn external_subtitle_path_roundtrips() {
        let store = JobStore::new(pool().await);
        let mut new = new_job();
        new.external_subtitle_path =
            Some(std::path::PathBuf::from("C:\\data\\uploads\\abc.srt"));
        let job = store.create(&new).await.unwrap();
        assert_eq!(
            job.external_subtitle_path.as_deref(),
            Some(std::path::Path::new("C:\\data\\uploads\\abc.srt"))
        );
        let got = store.get(&job.id).await.unwrap();
        assert_eq!(
            got.external_subtitle_path.as_deref(),
            Some(std::path::Path::new("C:\\data\\uploads\\abc.srt"))
        );
        // Normal jobs keep it None.
        let plain = store.create(&new_job()).await.unwrap();
        assert!(plain.external_subtitle_path.is_none());
    }

    #[tokio::test]
    async fn get_missing_is_not_found() {
        let store = JobStore::new(pool().await);
        match store.get("nope").await {
            Err(StoreError::NotFound(id)) => assert_eq!(id, "nope"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn status_transitions_enforced() {
        let store = JobStore::new(pool().await);
        let job = store.create(&new_job()).await.unwrap();

        // queued → running ok
        store.set_status(&job.id, JobStatus::Running).await.unwrap();
        // running → done ok
        store.set_status(&job.id, JobStatus::Done).await.unwrap();
        // done → running illegal
        assert!(matches!(
            store.set_status(&job.id, JobStatus::Running).await,
            Err(StoreError::Transition(_))
        ));
        // done → queued (retry) ok
        store.set_status(&job.id, JobStatus::Queued).await.unwrap();
    }

    #[tokio::test]
    async fn progress_and_output_persisted() {
        let store = JobStore::new(pool().await);
        let job = store.create(&new_job()).await.unwrap();
        store.set_progress(&job.id, 42, None).await.unwrap();
        assert_eq!(store.get(&job.id).await.unwrap().progress_pct, 42);
        store
            .set_progress(&job.id, 100, Some("C:\\media\\E01.de.srt"))
            .await
            .unwrap();
        let got = store.get(&job.id).await.unwrap();
        assert_eq!(got.progress_pct, 100);
        assert_eq!(got.output_path.as_deref(), Some(std::path::Path::new("C:\\media\\E01.de.srt")));
    }

    #[tokio::test]
    async fn log_tail_retains_last_bytes() {
        let store = JobStore::new(pool().await);
        let job = store.create(&new_job()).await.unwrap();
        // 200 lines × ~80 bytes ≈ 16KB, forces truncation.
        for i in 0..200 {
            store
                .append_log(&job.id, &format!("INFO: Translated batch 2.{i}: 90/240 lines (37%) padding-padding-padding"))
                .await
                .unwrap();
        }
        let tail = store.get(&job.id).await.unwrap().log_tail;
        assert!(tail.len() <= LOG_TAIL_MAX + 128, "tail should be bounded, got {}", tail.len());
        assert!(tail.contains("batch 2.199"), "tail must contain the last line");
        assert!(!tail.starts_with("batch 2.0"), "oldest lines must be dropped");
    }

    #[tokio::test]
    async fn log_tail_truncation_is_char_boundary_safe() {
        // Regression: truncating the log tail must not panic when the byte
        // offset lands inside a multi-byte UTF-8 character (Ukrainian Cyrillic).
        // append_log previously sliced by raw byte index.
        let store = JobStore::new(pool().await);
        let job = store.create(&new_job()).await.unwrap();
        let line = "перекладено субтитрів перевірка тексту українською мовою";
        for i in 0..500 {
            store.append_log(&job.id, &format!("{line} #{i}")).await.unwrap();
        }
        let tail = store.get(&job.id).await.unwrap().log_tail;
        assert!(tail.len() <= LOG_TAIL_MAX + 8, "tail should be bounded, got {}", tail.len());
        assert!(tail.contains("#499"), "tail must contain the last line");
    }

    #[tokio::test]
    async fn recovery_marks_running_interrupted_and_requeues() {
        let store = JobStore::new(pool().await);
        let a = store.create(&new_job()).await.unwrap();
        let b = store.create(&new_job()).await.unwrap();
        store.set_status(&a.id, JobStatus::Running).await.unwrap();

        let interrupted = store.recover_interrupted().await.unwrap();
        assert_eq!(interrupted, vec![a.id.clone()]);
        assert_eq!(store.get(&a.id).await.unwrap().status, JobStatus::Interrupted);

        let pending = store.pending_queued().await.unwrap();
        assert_eq!(pending.iter().map(|j| j.id.clone()).collect::<Vec<_>>(), vec![b.id.clone()]);
    }

    #[tokio::test]
    async fn list_orders_newest_first() {
        let store = JobStore::new(pool().await);
        let a = store.create(&new_job()).await.unwrap();
        let b = store.create(&new_job()).await.unwrap();
        let list = store.list(10).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, b.id);
        assert_eq!(list[1].id, a.id);
    }
}
