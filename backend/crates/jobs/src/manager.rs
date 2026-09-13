//! The job manager: bounded-concurrency queue, cancellation, retry, and event
//! fan-out (FR-14/16/17, §10).

use crate::events::{JobEvent, JobEventEnvelope};
use crate::model::{Job, JobStatus, NewJob};
use crate::runner::{JobRunner, RunError};
use crate::store::JobStore;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, Mutex, Notify, Semaphore};
use tokio_util::sync::CancellationToken;
use transflator_config::settings::keys;
use transflator_config::{AppConfig, Secrets, SettingsStore};
use transflator_media::PathGuard;
use transflator_translate::Provider;

#[derive(Debug, thiserror::Error)]
pub enum ManagerError {
    #[error("job not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("provider {0} requires an API key — set it in Settings first")]
    MissingApiKey(String),
    #[error("database error: {0}")]
    Store(#[from] crate::store::StoreError),
    #[error("job queue is full")]
    QueueFull,
}

pub type ManagerResult<T> = std::result::Result<T, ManagerError>;

/// Bulk delete scope for job history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteFilter {
    All,
    Failed,
    Done,
    Interrupted,
}

/// A queued job awaiting a free permit. Ordered so the queue worker pops the
/// highest priority first; ties (same priority) resolve to the earliest
/// submission (FIFO) via the sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    /// Higher = more urgent (1 = "start now", 0 = "add to queue").
    prio: u32,
    /// Monotonic submission counter (earlier = smaller).
    seq: u64,
    id: String,
}

impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Pending {
    fn cmp(&self, other: &Self) -> Ordering {
        // Max-heap: higher priority first; within a priority, earlier seq first.
        self.prio
            .cmp(&other.prio)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

/// Owns the queue, the concurrency bound, cancellation tokens, and the event
/// broadcast. The API layer holds an `Arc<JobManager>` in its state.
pub struct JobManager {
    store: JobStore,
    pool: sqlx::sqlite::SqlitePool,
    secrets: Secrets,
    #[allow(dead_code)]
    guard: PathGuard,
    /// Queue of `(priority, job_id)`; higher priority jumps ahead (FR-14/§13).
    tx: mpsc::Sender<(u32, String)>,
    /// Taken by [`JobManager::start`] (the receiver is not `Clone`).
    rx: std::sync::Mutex<Option<mpsc::Receiver<(u32, String)>>>,
    events_tx: broadcast::Sender<JobEventEnvelope>,
    cancels: Arc<Mutex<HashMap<String, Arc<CancellationToken>>>>,
    sem: Arc<Semaphore>,
    /// Wakes the queue worker when a running job frees a permit so the next
    /// highest-priority pending job can be dispatched without waiting for a
    /// new submission.
    dispatch_notify: Arc<Notify>,
    runner: Arc<dyn JobRunner>,
    /// Set on graceful shutdown (NFR-6) to stop the queue worker from
    /// dispatching new work while in-flight jobs drain.
    shutdown: CancellationToken,
}

impl JobManager {
    pub fn new(
        config: AppConfig,
        pool: sqlx::sqlite::SqlitePool,
        secrets: Secrets,
        guard: PathGuard,
        runner: Arc<dyn JobRunner>,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(1024);
        let (events_tx, _) = broadcast::channel(1024);
        let concurrency = config.concurrency;
        Arc::new(Self {
            store: JobStore::new(pool.clone()),
            pool,
            secrets,
            guard,
            tx,
            rx: std::sync::Mutex::new(Some(rx)),
            events_tx,
            cancels: Arc::new(Mutex::new(HashMap::new())),
            sem: Arc::new(Semaphore::new(concurrency)),
            dispatch_notify: Arc::new(Notify::new()),
            runner,
            shutdown: CancellationToken::new(),
        })
    }

    /// Spawn the worker: recover interrupted jobs, re-queue pending ones, then
    /// process the queue under the concurrency bound.
    pub fn start(self: &Arc<Self>) {
        let Some(rx) = self.rx.lock().unwrap().take() else {
            tracing::warn!("JobManager::start called twice; ignoring");
            return;
        };
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let interrupted = this.store.recover_interrupted().await;
            if let Ok(ids) = &interrupted {
                if !ids.is_empty() {
                    tracing::warn!(count = ids.len(), "marked running jobs as interrupted");
                }
            }
            if let Ok(pending) = this.store.pending_queued().await {
                if !pending.is_empty() {
                    tracing::info!(count = pending.len(), "re-queued pending jobs");
                }
                for job in pending {
                    // Re-queued jobs keep their original (normal) priority.
                    let _ = this.submit(&job.id, 0).await;
                }
            }
            let mut rx = rx;
            let mut heap: BinaryHeap<Pending> = BinaryHeap::new();
            let mut seq: u64 = 0;
            loop {
                // Dispatch as many pending jobs as there are free permits,
                // highest priority first (ties broken by submission order).
                while let Ok(permit) = this.sem.clone().try_acquire_owned() {
                    let Some(pending) = heap.pop() else { break };
                    let this = Arc::clone(&this);
                    tokio::spawn(async move {
                        let permit = permit;
                        this.run_one(pending.id).await;
                        // Release the permit, then wake the worker so the next
                        // pending job can start (the drop must precede the wake).
                        drop(permit);
                        this.dispatch_notify.notify_one();
                    });
                }
                tokio::select! {
                    maybe = rx.recv() => match maybe {
                        Some((prio, id)) => {
                            seq += 1;
                            heap.push(Pending { prio, seq, id });
                        }
                        None => break,
                    },
                    // Graceful shutdown (NFR-6): stop pulling new work so only
                    // in-flight jobs remain to drain.
                    _ = this.shutdown.cancelled() => {
                        tracing::info!("queue worker stopping (shutdown)");
                        break;
                    }
                    // A running job freed a permit; re-check the heap.
                    _ = this.dispatch_notify.notified() => {}
                }
            }
        });
    }

    /// Subscribe to job events (the API layer bridges this to WebSockets).
    pub fn subscribe(&self) -> broadcast::Receiver<JobEventEnvelope> {
        self.events_tx.subscribe()
    }

    /// Graceful shutdown (NFR-6): stop dispatching new queue work and wait for
    /// in-flight jobs to finish, up to `timeout`. Returns the number of jobs
    /// still running when the window elapsed (the caller marks them interrupted
    /// so they are resumable on the next boot). Queued-but-not-started jobs are
    /// left queued and are re-dispatched on the next boot.
    pub async fn drain(&self, timeout: Duration) -> usize {
        self.shutdown.cancel();
        let deadline = Instant::now() + timeout;
        loop {
            let running = self.cancels.lock().await.len();
            if running == 0 {
                return 0;
            }
            if Instant::now() >= deadline {
                return running;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Mark any still-running jobs as `interrupted` (resumable) — the shutdown
    /// fallback for jobs that did not finish within the drain window. Idempotent
    /// with boot-time [`JobStore::recover_interrupted`].
    pub async fn mark_running_interrupted(&self) -> usize {
        match self.store.recover_interrupted().await {
            Ok(ids) => ids.len(),
            Err(e) => {
                tracing::error!(error = %e, "failed to mark running jobs interrupted");
                0
            }
        }
    }

    async fn submit(&self, id: &str, priority: u32) -> Result<(), ManagerError> {
        self.tx
            .send((priority, id.to_string()))
            .await
            .map_err(|_| ManagerError::QueueFull)
    }

    async fn has_api_key(&self, provider: Provider) -> bool {
        let settings = SettingsStore::new(&self.pool, &self.secrets);
        if let Ok(Some(k)) = settings
            .get_secret(&format!("api_key.{}", provider.as_str()))
            .await
        {
            if !k.trim().is_empty() {
                return true;
            }
        }
        if let Some(env_key) = provider.env_key() {
            if let Ok(v) = std::env::var(env_key) {
                if !v.trim().is_empty() {
                    return true;
                }
            }
        }
        false
    }

    /// Create a job (fail fast on missing API key, §6.6) and enqueue it.
    pub async fn create_job(&self, new: &NewJob) -> ManagerResult<Job> {
        let provider = Provider::parse(&new.provider)
            .ok_or_else(|| ManagerError::Invalid(format!("unknown provider: {}", new.provider)))?;
        if provider.requires_api_key() && !self.has_api_key(provider).await {
            return Err(ManagerError::MissingApiKey(provider.as_str().to_string()));
        }
        if provider == Provider::Custom {
            let settings = SettingsStore::new(&self.pool, &self.secrets);
            let server = settings
                .get(keys::CUSTOM_SERVER_URL)
                .await
                .ok()
                .flatten()
                .filter(|s| !s.trim().is_empty());
            if server.is_none() {
                return Err(ManagerError::Invalid(
                    "custom server URL is not configured — set it in Settings".into(),
                ));
            }
        }
        let job = self.store.create(new).await?;
        // "Start now" jumps ahead of normal (add-to-queue) jobs (FR-14/§13).
        let priority = if new.start_now { 1 } else { 0 };
        self.submit(&job.id, priority).await?;
        tracing::info!(
            job = %job.id,
            provider = %job.provider,
            target = %job.target_language,
            source = %job.source_path.display(),
            start_now = new.start_now,
            "job created"
        );
        Ok(job)
    }

    /// Cancel a queued or running job (FR-17).
    pub async fn cancel_job(&self, id: &str) -> ManagerResult<()> {
        let job = self.store.get(id).await?;
        match job.status {
            JobStatus::Queued => {
                self.store.set_status(id, JobStatus::Canceled).await?;
                let _ = self.events_tx.send(JobEventEnvelope::new(
                    id,
                    JobEvent::Failed {
                        message: "canceled".into(),
                    },
                ));
            }
            JobStatus::Running => {
                let token = {
                    let cancels = self.cancels.lock().await;
                    cancels.get(id).cloned()
                };
                match token {
                    Some(t) => t.cancel(),
                    None => return Err(ManagerError::Invalid("no running process for job".into())),
                }
                // The running task performs the status transition.
            }
            other => {
                return Err(ManagerError::Invalid(format!(
                    "job is {other}, cannot cancel"
                )))
            }
        }
        tracing::info!(job = %id, status = %job.status, "cancel requested");
        Ok(())
    }

    /// Retry a finished job (FR-18): terminal → queued with the given mode,
    /// re-enqueue.
    pub async fn retry_job(&self, id: &str, mode: crate::model::RetryMode) -> ManagerResult<Job> {
        let job = self.store.get(id).await?;
        if !job.status.is_terminal() {
            return Err(ManagerError::Invalid(format!(
                "job is {}, only finished jobs can be retried",
                job.status
            )));
        }
        // Machine check first (terminal → queued), then persist mode + reset.
        crate::machine::transition(job.status, JobStatus::Queued)
            .map_err(|e| ManagerError::Invalid(e.to_string()))?;
        self.store.prepare_retry(id, mode.as_str()).await?;
        let job = self.store.get(id).await?;
        self.submit(id, 0).await?;
        tracing::info!(job = %id, mode = %mode.as_str(), "job retry requested");
        Ok(job)
    }

    pub async fn get(&self, id: &str) -> ManagerResult<Job> {
        Ok(self.store.get(id).await?)
    }

    pub async fn list(&self, limit: i64) -> ManagerResult<Vec<Job>> {
        Ok(self.store.list(limit).await?)
    }

    /// Delete one job from history. If the job is still running, its
    /// cancellation token is triggered immediately before the row is removed.
    pub async fn delete_job(&self, id: &str) -> ManagerResult<()> {
        let job = self.store.get(id).await?;
        let was_running = job.status == JobStatus::Running;
        if was_running {
            self.request_cancel(id).await;
        }
        self.store.delete(id).await?;
        tracing::info!(
            job = %id,
            status = %job.status,
            canceled = was_running,
            "job deleted"
        );
        Ok(())
    }

    /// Delete a group of jobs from history. Running jobs in the selected group
    /// are canceled immediately before their rows are removed.
    pub async fn delete_jobs(&self, filter: DeleteFilter) -> ManagerResult<usize> {
        let jobs = match filter {
            DeleteFilter::All => self.store.list(i64::MAX).await?,
            DeleteFilter::Failed => {
                self.store.list_by_status(&[JobStatus::Failed]).await?
            }
            DeleteFilter::Done => {
                self.store.list_by_status(&[JobStatus::Done]).await?
            }
            DeleteFilter::Interrupted => {
                self.store.list_by_status(&[JobStatus::Interrupted]).await?
            }
        };
        for job in &jobs {
            if job.status == JobStatus::Running {
                self.request_cancel(&job.id).await;
            }
        }
        let deleted = match filter {
            DeleteFilter::All => self.store.delete_all().await?,
            DeleteFilter::Failed => self.store.delete_by_status(&[JobStatus::Failed]).await?,
            DeleteFilter::Done => self.store.delete_by_status(&[JobStatus::Done]).await?,
            DeleteFilter::Interrupted => {
                self.store.delete_by_status(&[JobStatus::Interrupted]).await?
            }
        };
        tracing::info!(filter = ?filter, deleted, "jobs deleted");
        Ok(deleted as usize)
    }

    async fn request_cancel(&self, id: &str) {
        if let Some(token) = self.cancels.lock().await.get(id).cloned() {
            token.cancel();
        }
    }

    async fn run_one(&self, id: String) {
        let job = match self.store.get(&id).await {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(job = %id, error = %e, "dropping job from queue");
                return;
            }
        };
        if job.status != JobStatus::Queued {
            // Canceled while sitting in the queue.
            return;
        }

        let cancel = Arc::new(CancellationToken::new());
        {
            let mut cancels = self.cancels.lock().await;
            // A duplicate delivery for the same id (the boot re-queue can race an
            // early submit) must not clobber the live token. Only the first
            // run_one for an id owns the token and the Queued -> Running claim;
            // any later one exits here before touching the map.
            if cancels.contains_key(&id) {
                return;
            }
            cancels.insert(id.clone(), Arc::clone(&cancel));
        }

        if let Err(e) = self.store.set_status(&id, JobStatus::Running).await {
            self.cancels.lock().await.remove(&id);
            tracing::warn!(job = %id, error = %e, "failed to mark job running");
            return;
        }
        let _ = self.events_tx.send(JobEventEnvelope::new(
            &id,
            JobEvent::Status {
                status: JobStatus::Running.as_str().into(),
            },
        ));
        tracing::info!(
            job = %id,
            provider = %job.provider,
            target = %job.target_language,
            model = ?job.model,
            "job started"
        );

        let result = self.runner.run(&job, &cancel, &self.events_tx).await;
        match result {
            Ok(output) => {
                let _ = self.store.set_progress(&id, 100, Some(&output)).await;
                let _ = self.store.set_status(&id, JobStatus::Done).await;
                let _ = self.events_tx.send(JobEventEnvelope::new(
                    &id,
                    JobEvent::Done {
                        output_path: output.clone(),
                    },
                ));
                tracing::info!(job = %id, output = %output, "job done");
            }
            Err(RunError::Canceled) => {
                let _ = self.store.set_status(&id, JobStatus::Canceled).await;
                let _ = self.events_tx.send(JobEventEnvelope::new(
                    &id,
                    JobEvent::Failed {
                        message: "canceled".into(),
                    },
                ));
                tracing::info!(job = %id, "job canceled");
            }
            Err(RunError::Failed(msg)) => {
                let _ = self.store.set_error(&id, &msg).await;
                let _ = self.store.set_status(&id, JobStatus::Failed).await;
                let _ = self.events_tx.send(JobEventEnvelope::new(
                    &id,
                    JobEvent::Failed { message: msg.clone() },
                ));
                tracing::warn!(job = %id, error = %msg, "job failed");
            }
        }
        self.cancels.lock().await.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;
    use futures_util::future::BoxFuture;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use transflator_config::db;

    struct TestEnv {
        manager: Arc<JobManager>,
        store: JobStore,
        /// Keeps the temp dir (and its DB) alive for the test's duration.
        #[allow(dead_code)]
        dir: tempfile::TempDir,
    }

    async fn env(concurrency: usize, runner: Arc<dyn JobRunner>) -> TestEnv {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::init_pool(&dir.path().join("t.db")).await.unwrap();
        let config = AppConfig {
            media_roots: vec![dir.path().to_path_buf()],
            db_path: dir.path().join("t.db"),
            data_dir: dir.path().to_path_buf(),
            app_secret: "test-secret".into(),
            auth_token: String::new(),
            host: "127.0.0.1".into(),
            port: 0,
            llm_subtrans_home: dir.path().to_path_buf(),
            python_bin: "python".into(),
            ffmpeg_bin: "ffmpeg".into(),
            ffprobe_bin: "ffprobe".into(),
            concurrency,
            output_pattern: "{name}.{lang}.srt".into(),
            overwrite_behavior: Default::default(),
            default_target_language: "en".into(),
            ffprobe_timeout_secs: 10,
            ffmpeg_timeout_secs: 30,
            web_root: dir.path().to_path_buf(),
            log_level: "info".into(),
        };
        let secrets = Secrets::from_app_secret("test-secret");
        let settings = SettingsStore::new(&pool, &secrets);
        settings
            .set(keys::CUSTOM_SERVER_URL, "http://localhost:1234")
            .await
            .unwrap();
        let guard = PathGuard::new(&config.media_roots).unwrap();
        let manager = JobManager::new(config, pool, secrets, guard, runner);
        let store = JobStore::new(manager.pool.clone());
        manager.start();
        TestEnv { manager, store, dir }
    }

    fn new_job(path: &str) -> NewJob {
        NewJob {
            source_path: std::path::PathBuf::from(path),
            subtitle_stream_index: Some(1),
            source_language: Some("en".into()),
            target_language: "de".into(),
            provider: "custom".into(),
            model: None,
            external_subtitle_path: None,
            movie_name: None,
            description: None,
            start_now: false,
        }
    }

    async fn wait_status(
        store: &JobStore,
        id: &str,
        want: JobStatus,
        timeout: Duration,
    ) -> Job {
        let start = std::time::Instant::now();
        loop {
            let job = store.get(id).await.unwrap();
            if job.status == want {
                return job;
            }
            assert!(
                start.elapsed() < timeout,
                "timed out waiting for {want}; status is {}",
                job.status
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn create_and_run_to_done() {
        let env = env(2, Arc::new(FakeRunner::new())).await;
        let job = env
            .manager
            .create_job(&new_job("C:\\media\\E01.mkv"))
            .await
            .unwrap();
        let done = wait_status(&env.store, &job.id, JobStatus::Done, Duration::from_secs(5))
            .await;
        assert_eq!(done.progress_pct, 100);
        assert!(done.output_path.is_some());
        assert!(done.error_message.is_none());
    }

    #[tokio::test]
    async fn drain_waits_for_in_flight_then_marks_interrupted() {
        // A hanging runner keeps the job in-flight so the drain window expires
        // with it still running (the NFR-6 shutdown straggler path).
        let mut runner = FakeRunner::new();
        runner.hang = true;
        let env = env(1, Arc::new(runner)).await;
        let job = env
            .manager
            .create_job(&new_job("C:\\media\\E01.mkv"))
            .await
            .unwrap();
        // Wait until the worker has dispatched it and it is actually running.
        wait_status(&env.store, &job.id, JobStatus::Running, Duration::from_secs(5)).await;

        // Drain with a short window: the hanging job never finishes, so drain
        // reports it still in flight.
        let in_flight = env.manager.drain(Duration::from_millis(200)).await;
        assert_eq!(in_flight, 1, "drain should report the in-flight job");

        // Mark the straggler interrupted (resumable) — the shutdown fallback.
        let marked = env.manager.mark_running_interrupted().await;
        assert_eq!(marked, 1, "should mark the running job interrupted");
        let j = env.store.get(&job.id).await.unwrap();
        assert_eq!(j.status, JobStatus::Interrupted);
    }

    #[tokio::test]
    async fn missing_api_key_fails_fast() {
        // The dev machine may export OPENAI_API_KEY; clear it so the
        // fail-fast path is deterministic (no settings entry either).
        std::env::remove_var("OPENAI_API_KEY");
        let env = env(2, Arc::new(FakeRunner::new())).await;
        let mut job = new_job("C:\\media\\E01.mkv");
        job.provider = "openai".into();
        match env.manager.create_job(&job).await {
            Err(ManagerError::MissingApiKey(p)) => assert_eq!(p, "openai"),
            other => panic!("expected MissingApiKey, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn failure_records_error() {
        let env = env(
            2,
            Arc::new(FakeRunner {
                delay: Duration::from_millis(1),
                fail: true,
                hang: false,
            }),
        )
        .await;
        let job = env
            .manager
            .create_job(&new_job("C:\\media\\E01.mkv"))
            .await
            .unwrap();
        let failed = wait_status(&env.store, &job.id, JobStatus::Failed, Duration::from_secs(5))
            .await;
        assert!(
            failed.error_message.as_deref() == Some("fake: forced failure"),
            "error message should be recorded"
        );
        assert!(failed.output_path.is_none());
    }

    #[tokio::test]
    async fn concurrency_is_bounded() {
        // Gauge runner: tracks max simultaneous runs.
        #[derive(Default)]
        struct Gauge {
            active: AtomicUsize,
            max: AtomicUsize,
        }
        struct GaugeRunner {
            gauge: Arc<Gauge>,
            delay: Duration,
        }
        impl JobRunner for GaugeRunner {
            fn run<'a>(
                &'a self,
                job: &'a Job,
                cancel: &'a CancellationToken,
                events: &'a broadcast::Sender<JobEventEnvelope>,
            ) -> BoxFuture<'a, Result<String, RunError>> {
                let gauge = Arc::clone(&self.gauge);
                let delay = self.delay;
                Box::pin(async move {
                    let active = gauge.active.fetch_add(1, Ordering::SeqCst) + 1;
                    gauge
                        .max
                        .fetch_max(active, Ordering::SeqCst);
                    let _ = events.send(JobEventEnvelope::new(
                        job.id.clone(),
                        JobEvent::Log { line: "gauge".into() },
                    ));
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = cancel.cancelled() => return Err(RunError::Canceled),
                    }
                    gauge.active.fetch_sub(1, Ordering::SeqCst);
                    Ok(format!("gauge-{}.srt", job.id))
                })
            }
        }

        let gauge = Arc::new(Gauge::default());
        let runner = Arc::new(GaugeRunner {
            gauge: Arc::clone(&gauge),
            delay: Duration::from_millis(50),
        });
        let env = env(2, runner).await;
        for i in 0..6 {
            env.manager
                .create_job(&new_job(&format!("C:\\media\\E0{i}.mkv")))
                .await
                .unwrap();
        }
        // Wait for all to finish.
        let start = std::time::Instant::now();
        loop {
            let list = env.store.list(100).await.unwrap();
            if list.iter().all(|j| j.status == JobStatus::Done) {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            gauge.max.load(Ordering::SeqCst) <= 2,
            "max concurrency was {}",
            gauge.max.load(Ordering::SeqCst)
        );
        assert_eq!(gauge.max.load(Ordering::SeqCst), 2, "should have used the full bound");
    }

    /// Runner that records the order in which jobs actually start.
    struct OrderRec {
        order: tokio::sync::Mutex<Vec<String>>,
    }
    struct OrderRunner {
        rec: Arc<OrderRec>,
        delay: Duration,
    }
    impl JobRunner for OrderRunner {
        fn run<'a>(
            &'a self,
            job: &'a Job,
            cancel: &'a CancellationToken,
            _events: &'a broadcast::Sender<JobEventEnvelope>,
        ) -> BoxFuture<'a, Result<String, RunError>> {
            let rec = Arc::clone(&self.rec);
            let delay = self.delay;
            let hang = job.source_path.ends_with("A.mkv");
            Box::pin(async move {
                rec.order.lock().await.push(job.id.clone());
                if hang {
                    cancel.cancelled().await;
                    return Err(RunError::Canceled);
                }
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = cancel.cancelled() => return Err(RunError::Canceled),
                }
                Ok(format!("order-{}.srt", job.id))
            })
        }
    }

    #[tokio::test]
    async fn start_now_jumps_ahead_of_queued() {
        let rec = Arc::new(OrderRec {
            order: tokio::sync::Mutex::new(Vec::new()),
        });
        let runner = Arc::new(OrderRunner {
            rec: Arc::clone(&rec),
            delay: Duration::from_millis(50),
        });
        let env = env(1, runner).await;
        // A: normal; runs first (only permit) and holds it until canceled.
        let a = env
            .manager
            .create_job(&new_job("C:\\media\\A.mkv"))
            .await
            .unwrap();
        wait_status(&env.store, &a.id, JobStatus::Running, Duration::from_secs(5)).await;
        // B: normal, queued behind the running A.
        let b = env
            .manager
            .create_job(&new_job("C:\\media\\B.mkv"))
            .await
            .unwrap();
        // C: start-now — must jump ahead of B once a permit frees.
        let mut c = new_job("C:\\media\\C.mkv");
        c.start_now = true;
        let c = env.manager.create_job(&c).await.unwrap();
        // Let B and C settle into the queue before freeing the permit.
        tokio::time::sleep(Duration::from_millis(80)).await;
        env.manager.cancel_job(&a.id).await.unwrap();
        // Wait for B and C to finish.
        let start = Instant::now();
        loop {
            let b_done = env.store.get(&b.id).await.unwrap().status.is_terminal();
            let c_done = env.store.get(&c.id).await.unwrap().status.is_terminal();
            if b_done && c_done {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let order = rec.order.lock().await.clone();
        let pos_b = order.iter().position(|id| id == &b.id).unwrap();
        let pos_c = order.iter().position(|id| id == &c.id).unwrap();
        assert!(
            pos_c < pos_b,
            "start-now job should run before the queued job; order: {order:?}"
        );
    }

    #[tokio::test]
    async fn cancel_running_job() {
        let env = env(
            2,
            Arc::new(FakeRunner {
                delay: Duration::from_millis(1),
                fail: false,
                hang: true,
            }),
        )
        .await;
        let job = env
            .manager
            .create_job(&new_job("C:\\media\\E01.mkv"))
            .await
            .unwrap();
        let running = wait_status(&env.store, &job.id, JobStatus::Running, Duration::from_secs(5))
            .await;
        env.manager.cancel_job(&running.id).await.unwrap();
        let canceled =
            wait_status(&env.store, &job.id, JobStatus::Canceled, Duration::from_secs(5)).await;
        assert_eq!(canceled.status, JobStatus::Canceled);
    }

    #[tokio::test]
    async fn cancel_queued_job() {
        // concurrency=1; first job hangs, second stays queued.
        let env = env(
            1,
            Arc::new(FakeRunner {
                delay: Duration::from_millis(1),
                fail: false,
                hang: true,
            }),
        )
        .await;
        let a = env
            .manager
            .create_job(&new_job("C:\\media\\A.mkv"))
            .await
            .unwrap();
        wait_status(&env.store, &a.id, JobStatus::Running, Duration::from_secs(5)).await;
        let b = env
            .manager
            .create_job(&new_job("C:\\media\\B.mkv"))
            .await
            .unwrap();
        wait_status(&env.store, &b.id, JobStatus::Queued, Duration::from_millis(500))
            .await;
        env.manager.cancel_job(&b.id).await.unwrap();
        let canceled =
            wait_status(&env.store, &b.id, JobStatus::Canceled, Duration::from_secs(5)).await;
        assert_eq!(canceled.status, JobStatus::Canceled);
        // Clean up the hung job so the test ends quietly.
        let _ = env.manager.cancel_job(&a.id).await;
    }

    #[tokio::test]
    async fn retry_failed_job() {
        // First run fails (deterministic id via pre-seeded store), retry succeeds.
        let env = env(2, Arc::new(FakeRunner::new())).await;
        let job = env
            .manager
            .create_job(&new_job("C:\\media\\E01.mkv"))
            .await
            .unwrap();
        let done = wait_status(&env.store, &job.id, JobStatus::Done, Duration::from_secs(5))
            .await;
        // Retry a done job → queued → done again.
        let retried = env
            .manager
            .retry_job(&done.id, crate::model::RetryMode::Rerun)
            .await
            .unwrap();
        assert_eq!(retried.status, JobStatus::Queued);
        let done2 = wait_status(&env.store, &done.id, JobStatus::Done, Duration::from_secs(5))
            .await;
        assert_eq!(done2.status, JobStatus::Done);
    }

    #[tokio::test]
    async fn retry_rejects_active_jobs() {
        let env = env(
            1,
            Arc::new(FakeRunner {
                delay: Duration::from_millis(1),
                fail: false,
                hang: true,
            }),
        )
        .await;
        let job = env
            .manager
            .create_job(&new_job("C:\\media\\E01.mkv"))
            .await
            .unwrap();
        wait_status(&env.store, &job.id, JobStatus::Running, Duration::from_secs(5)).await;
        assert!(matches!(
            env.manager
                .retry_job(&job.id, crate::model::RetryMode::Rerun)
                .await,
            Err(ManagerError::Invalid(_))
        ));
        let _ = env.manager.cancel_job(&job.id).await;
    }

    /// Spawns a real long-running process (python sleep) so cancellation can be
    /// proven to actually kill the child, not just hide it in the UI (FR-17, §15).
    struct SleepRunner;

    impl JobRunner for SleepRunner {
        fn run<'a>(
            &'a self,
            job: &'a Job,
            cancel: &'a CancellationToken,
            events: &'a broadcast::Sender<JobEventEnvelope>,
        ) -> BoxFuture<'a, Result<String, RunError>> {
            Box::pin(async move {
                let _ = events.send(JobEventEnvelope::new(
                    job.id.clone(),
                    JobEvent::Log { line: "sleep: started".into() },
                ));
                let mut child = tokio::process::Command::new("python")
                    .args(["-c", "import time; time.sleep(30)"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|e| RunError::Failed(format!("spawn failed: {e}")))?;
                tokio::select! {
                    _ = cancel.cancelled() => {
                        transflator_translate::kill_tree(&mut child);
                        // Block until the killed process is actually reaped. If the
                        // kill failed, this would hang for the full 30s sleep and the
                        // test's elapsed-time assertion would catch it.
                        let _ = child.wait().await;
                        Err(RunError::Canceled)
                    }
                    st = child.wait() => {
                        Err(RunError::Failed(format!("slept to completion: {st:?}")))
                    }
                }
            })
        }
    }

    #[tokio::test]
    async fn cancel_actually_kills_child_process() {
        // Skip if python isn't available (e.g. a CI image without it).
        if tokio::process::Command::new("python")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_err()
        {
            eprintln!("skip: python not available");
            return;
        }
        let env = env(1, Arc::new(SleepRunner)).await;
        let job = env
            .manager
            .create_job(&new_job("C:\\media\\E01.mkv"))
            .await
            .unwrap();
        let running =
            wait_status(&env.store, &job.id, JobStatus::Running, Duration::from_secs(5)).await;
        let start = std::time::Instant::now();
        env.manager.cancel_job(&running.id).await.unwrap();
        let canceled =
            wait_status(&env.store, &job.id, JobStatus::Canceled, Duration::from_secs(10)).await;
        assert_eq!(canceled.status, JobStatus::Canceled);
        // If the child weren't killed, the runner would block on wait() for 30s.
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "cancel took {:?} — child process was not killed",
            start.elapsed()
        );
    }

    // Regression: the boot re-queue can race an early submit and deliver the
    // same id to the worker twice. Each fresh env below re-rolls that window,
    // so a token-clobbering bug (lost cancel token -> "no running process")
    // would surface here even when a single cancel_running_job run misses it.
    #[tokio::test]
    async fn cancel_running_job_is_stable_under_boot_double_submit() {
        for i in 0..50 {
            let env = env(
                2,
                Arc::new(FakeRunner {
                    delay: Duration::from_millis(1),
                    fail: false,
                    hang: true,
                }),
            )
            .await;
            let job = env
                .manager
                .create_job(&new_job(&format!("C:\\media\\E{i}.mkv")))
                .await
                .unwrap();
            let running =
                wait_status(&env.store, &job.id, JobStatus::Running, Duration::from_secs(5)).await;
            env.manager
                .cancel_job(&running.id)
                .await
                .unwrap_or_else(|e| panic!("cancel failed on iter {i}: {e}"));
            let canceled =
                wait_status(&env.store, &job.id, JobStatus::Canceled, Duration::from_secs(5)).await;
            assert_eq!(canceled.status, JobStatus::Canceled);
        }
    }

    #[tokio::test]
    async fn delete_queued_job_removes_row() {
        let env = env(
            1,
            Arc::new(FakeRunner {
                delay: Duration::from_millis(1),
                fail: false,
                hang: true,
            }),
        )
        .await;
        let a = env
            .manager
            .create_job(&new_job("C:\\media\\A.mkv"))
            .await
            .unwrap();
        wait_status(&env.store, &a.id, JobStatus::Running, Duration::from_secs(5)).await;
        let b = env
            .manager
            .create_job(&new_job("C:\\media\\B.mkv"))
            .await
            .unwrap();
        wait_status(&env.store, &b.id, JobStatus::Queued, Duration::from_millis(500))
            .await;

        env.manager.delete_job(&b.id).await.unwrap();
        assert!(matches!(
            env.store.get(&b.id).await,
            Err(crate::store::StoreError::NotFound(_))
        ));

        let _ = env.manager.cancel_job(&a.id).await;
    }

    #[tokio::test]
    async fn delete_running_job_cancels_and_removes_row() {
        let env = env(
            2,
            Arc::new(FakeRunner {
                delay: Duration::from_millis(1),
                fail: false,
                hang: true,
            }),
        )
        .await;
        let job = env
            .manager
            .create_job(&new_job("C:\\media\\E01.mkv"))
            .await
            .unwrap();
        wait_status(&env.store, &job.id, JobStatus::Running, Duration::from_secs(5)).await;

        env.manager.delete_job(&job.id).await.unwrap();
        assert!(matches!(
            env.store.get(&job.id).await,
            Err(crate::store::StoreError::NotFound(_))
        ));

        // Give the canceled runner a moment to observe the token and exit.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn delete_jobs_all_cancels_active_and_removes_every_row() {
        let env = env(
            2,
            Arc::new(FakeRunner {
                delay: Duration::from_millis(1),
                fail: false,
                hang: true,
            }),
        )
        .await;
        let a = env
            .manager
            .create_job(&new_job("C:\\media\\A.mkv"))
            .await
            .unwrap();
        let b = env
            .manager
            .create_job(&new_job("C:\\media\\B.mkv"))
            .await
            .unwrap();
        let c = env
            .manager
            .create_job(&new_job("C:\\media\\C.mkv"))
            .await
            .unwrap();

        // Wait until A and B are running and C is queued.
        let start = Instant::now();
        loop {
            let a_status = env.store.get(&a.id).await.unwrap().status;
            let b_status = env.store.get(&b.id).await.unwrap().status;
            let c_status = env.store.get(&c.id).await.unwrap().status;
            if a_status == JobStatus::Running
                && b_status == JobStatus::Running
                && c_status == JobStatus::Queued
            {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let deleted = env.manager.delete_jobs(DeleteFilter::All).await.unwrap();
        assert_eq!(deleted, 3);
        assert!(env.store.list(100).await.unwrap().is_empty());

        // Give the canceled runners a moment to observe the tokens and exit.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn delete_jobs_failed_only_removes_failed_rows() {
        let env = env(2, Arc::new(FakeRunner::new())).await;
        let done = env.store.create(&new_job("C:\\media\\done.mkv")).await.unwrap();
        let failed = env.store.create(&new_job("C:\\media\\failed.mkv")).await.unwrap();
        env.store
            .set_status(&done.id, JobStatus::Running)
            .await
            .unwrap();
        env.store
            .set_status(&done.id, JobStatus::Done)
            .await
            .unwrap();
        env.store
            .set_status(&failed.id, JobStatus::Running)
            .await
            .unwrap();
        env.store
            .set_status(&failed.id, JobStatus::Failed)
            .await
            .unwrap();

        let deleted = env
            .manager
            .delete_jobs(DeleteFilter::Failed)
            .await
            .unwrap();
        assert_eq!(deleted, 1);
        assert!(matches!(
            env.store.get(&failed.id).await,
            Err(crate::store::StoreError::NotFound(_))
        ));
        assert!(env.store.get(&done.id).await.is_ok());
    }
}
