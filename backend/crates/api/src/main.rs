//! TranSFlator server entrypoint.
//!
//! Wires configuration, logging, the SQLite pool, the path guard, and the
//! axum router together and runs until SIGTERM, draining gracefully.

mod auth;
mod error;
mod routes;
mod state;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::Json;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_appender::rolling::{RollingFileAppender, Rotation};

use transflator_config::{AppConfig, Secrets, SettingsStore};
use transflator_jobs::{JobManager, PipelineRunner};
use transflator_media::PathGuard;

use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut config = AppConfig::from_env();

    // Structured logging (NFR-6): rolling files live next to the database.
    let logs_dir = logs_dir_for(&config.db_path);
    std::fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create logs dir {}", logs_dir.display()))?;
    let logs_dir = std::fs::canonicalize(&logs_dir)
        .with_context(|| format!("failed to canonicalize logs dir {}", logs_dir.display()))?;
    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("transflator.log")
        .max_log_files(15)
        .build(&logs_dir)
        .map_err(|e| anyhow::anyhow!("failed to initialise log appender: {e}"))?;
    let (non_blocking, _log_guard) = tracing_appender::non_blocking(file_appender);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(false)
        .with_writer(non_blocking)
        .init();

    info!(
        dir = %logs_dir.display(),
        "log file initialised"
    );

    info!(
        roots = ?config.media_roots,
        db = %config.db_path.display(),
        port = config.port,
        "starting TranSFlator"
    );

    if config.using_default_secret() {
        warn!("APP_SECRET is using the insecure default — set a strong value for production");
    }

    // Boot validation (§6.12): fail fast, naming the missing dependency.
    match transflator_config::boot::validate(&config) {
        Ok(report) => {
            info!(
                readable_roots = report.readable_roots,
                total_roots = report.total_roots,
                "configuration validated"
            )
        }
        Err(problems) => {
            for p in &problems {
                tracing::error!(problem = %p, "boot validation failed");
            }
            anyhow::bail!(
                "boot validation failed with {} problem(s); refusing to start",
                problems.len()
            );
        }
    }

    let secrets = Arc::new(Secrets::from_app_secret(&config.app_secret));

    // Path guard (§6.9): canonicalizes media roots and rejects traversal.
    let guard = Arc::new(
        PathGuard::new(&config.media_roots)
            .inspect_err(|e| tracing::error!(error = %e, "failed to build path guard"))?,
    );
    if !guard.has_readable_root() {
        warn!("no readable media root configured — library browsing will be empty");
    }

    // Database (NFR-1).
    let pool = transflator_config::db::init_pool(&config.db_path)
        .await
        .inspect_err(|e| tracing::error!(error = %e, "failed to initialise database"))?;
    info!(db = %config.db_path.display(), "database ready");

    // Apply the stored concurrency override (Settings) before the manager fixes
    // the concurrency bound at construction.
    {
        let store = SettingsStore::new(&pool, secrets.as_ref());
        if let Ok(eff) = store.effective(&config).await {
            config.concurrency = eff.concurrency;
        }
    }
    info!(
        concurrency = config.concurrency,
        "effective concurrency loaded"
    );

    // Job pipeline + queue (Phase 4): probe → extract → llm-subtrans.
    let runner = Arc::new(PipelineRunner {
        config: config.clone(),
        secrets: secrets.as_ref().clone(),
        pool: pool.clone(),
        guard: guard.as_ref().clone(),
    });
    let manager = JobManager::new(
        config.clone(),
        pool.clone(),
        secrets.as_ref().clone(),
        guard.as_ref().clone(),
        runner,
    );

    // Shared HTTP client for provider model discovery (15s timeout).
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("TranSFlator/0.1")
        .build()?;

    let state = Arc::new(AppState {
        config: Arc::new(config.clone()),
        logs_dir: logs_dir.clone(),
        pool,
        secrets,
        guard,
        manager: Arc::clone(&manager),
        http,
    });

    // Spawn the queue worker (recovers interrupted jobs, re-queues pending).
    manager.start();
    info!("job queue started");

    let app = build_router(state.clone());

    let ip: std::net::IpAddr = config.host.parse()?;
    let addr = SocketAddr::from((ip, config.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "listening");

    // Graceful shutdown on SIGTERM/SIGINT (NFR-6): stop the queue, drain
    // in-flight jobs for up to the window, then mark any stragglers interrupted
    // so they are resumable on the next boot. On Unix we watch both SIGINT
    // (Ctrl+C) and SIGTERM (docker stop / kill); on Windows only Ctrl+C exists.
    let shutdown_manager = Arc::clone(&manager);
    let shutdown = async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut term =
                signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = term.recv() => {},
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let window = drain_timeout();
        info!(
            window_secs = window.as_secs(),
            "shutdown signal received; draining in-flight jobs"
        );
        let in_flight = shutdown_manager.drain(window).await;
        if in_flight > 0 {
            let marked = shutdown_manager.mark_running_interrupted().await;
            info!(
                in_flight,
                marked, "in-flight jobs did not finish in time; marked interrupted (resumable)"
            );
        }
        info!("drain complete");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;

    info!("shutdown complete");
    Ok(())
}

fn build_router(state: Arc<AppState>) -> axum::Router {
    let cors = tower_http::cors::CorsLayer::permissive();

    let api = axum::Router::new()
        .route("/healthz", get(healthz))
        .route("/health/logs", get(routes::health::list))
        .route("/health/logs/:name", get(routes::health::download))
        .route("/library/roots", get(routes::library::roots))
        .route("/library/tree", get(routes::library::tree))
        .route("/library/search", get(routes::library::search))
        .route("/media/streams", get(routes::media::streams))
        .route("/media/file", get(routes::media::file))
        .route("/media/thumbnail", get(routes::media::thumbnail))
        .route("/models", get(routes::models::list))
        .route(
            "/jobs",
            post(routes::jobs::create)
                .get(routes::jobs::list)
                .delete(routes::jobs::delete_many),
        )
        .route("/jobs/upload", post(routes::jobs::upload))
        .route(
            "/jobs/:id",
            get(routes::jobs::get).delete(routes::jobs::delete),
        )
        .route("/jobs/:id/cancel", post(routes::jobs::cancel))
        .route("/jobs/:id/retry", post(routes::jobs::retry))
        .route("/jobs/:id/download", get(routes::jobs::download))
        .route("/jobs/:id/events", get(routes::jobs::events))
        .route(
            "/settings",
            get(routes::settings::get).put(routes::settings::put),
        )
        .route("/settings/plex/connect", post(routes::plex::connect))
        .route("/settings/plex/connect/:pin_id", get(routes::plex::poll))
        .route("/settings/plex/servers", get(routes::plex::servers))
        .route(
            "/settings/plex/test",
            post(routes::media_servers::test_plex),
        )
        .route("/settings/plex", delete(routes::media_servers::delete_plex))
        .route(
            "/settings/jellyfin/test",
            post(routes::media_servers::test_jellyfin),
        )
        .route(
            "/settings/jellyfin",
            delete(routes::media_servers::delete_jellyfin),
        )
        .with_state(state.clone());

    let mut router = axum::Router::new()
        .nest("/api", api)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new());

    // Auth (NFR-7): protect /api/* (SPA static files + /healthz stay open).
    router = router.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        auth::require_token,
    ));

    // Serve the built frontend if the web root exists; otherwise a placeholder.
    let web_root = &state.config.web_root;
    if web_root.join("index.html").exists() {
        info!(root = %web_root.display(), "serving frontend");
        let dir =
            ServeDir::new(web_root).not_found_service(ServeFile::new(web_root.join("index.html")));
        router = router.fallback_service(dir);
    } else {
        warn!(root = %web_root.display(), "web root missing index.html; frontend not served");
        router = router.fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                "TranSFlator API is running. Frontend build not found at web root.",
            )
        });
    }

    router
}

async fn healthz() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "transflator",
        "time": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }))
}

/// How long graceful shutdown waits for in-flight jobs to finish before marking
/// the stragglers interrupted (NFR-6).
fn drain_timeout() -> Duration {
    Duration::from_secs(30)
}

/// Log files are stored in a `logs` folder next to the SQLite database file.
fn logs_dir_for(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .map(|parent| parent.join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"))
}
