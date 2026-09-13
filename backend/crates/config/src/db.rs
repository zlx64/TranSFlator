//! SQLite pool creation and schema migrations.
//!
//! Uses WAL mode and a small connection pool. The schema matches §9 of the
//! project prompt: `media_cache`, `jobs`, `settings`.

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

/// The SQL schema, applied idempotently at boot.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS media_cache (
    path         TEXT PRIMARY KEY,
    size         INTEGER NOT NULL,
    mtime        INTEGER NOT NULL,
    duration_s   REAL,
    container    TEXT,
    streams_json TEXT NOT NULL,
    scanned_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
    id                    TEXT PRIMARY KEY,
    source_path           TEXT NOT NULL,
    subtitle_stream_index INTEGER,
    source_language       TEXT,
    target_language       TEXT NOT NULL,
    provider              TEXT NOT NULL,
    model                 TEXT,
    status                TEXT NOT NULL,
    progress_pct          INTEGER NOT NULL DEFAULT 0,
    output_path           TEXT,
    project_file_path     TEXT,
    external_subtitle_path TEXT,
    log_tail              TEXT,
    error_message         TEXT,
    retry_mode            TEXT NOT NULL DEFAULT 'rerun',
    movie_name            TEXT,
    description           TEXT,
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_jobs_status  ON jobs(status);
CREATE INDEX IF NOT EXISTS idx_jobs_created ON jobs(created_at);

CREATE TABLE IF NOT EXISTS settings (
    key       TEXT PRIMARY KEY,
    value     TEXT NOT NULL,
    is_secret INTEGER NOT NULL DEFAULT 0
);
"#;

/// Open (creating if necessary) the SQLite database and apply migrations.
pub async fn init_pool(path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
    let options = SqliteConnectOptions::from_str(&url)?
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;

    migrate(&pool).await?;
    Ok(pool)
}

/// Apply the schema (idempotent), plus column-level migrations for databases
/// created before a column existed.
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(SCHEMA).execute(pool).await?;
    migrate_columns(pool).await?;
    Ok(())
}

/// Add columns that were introduced after the initial release.
async fn migrate_columns(pool: &SqlitePool) -> anyhow::Result<()> {
    // jobs.retry_mode (Phase 5, FR-18).
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pragma_table_info('jobs') WHERE name = 'retry_mode'",
    )
    .fetch_one(pool)
    .await?;
    if count == 0 {
        sqlx::query("ALTER TABLE jobs ADD COLUMN retry_mode TEXT NOT NULL DEFAULT 'rerun'")
            .execute(pool)
            .await?;
    }
    // jobs.external_subtitle_path (Phase 7, §6.1): uploaded .srt/.ass/.vtt used
    // instead of an extracted stream; NULL for normal extraction jobs.
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pragma_table_info('jobs') WHERE name = 'external_subtitle_path'",
    )
    .fetch_one(pool)
    .await?;
    if count == 0 {
        sqlx::query("ALTER TABLE jobs ADD COLUMN external_subtitle_path TEXT")
            .execute(pool)
            .await?;
    }
    // jobs.movie_name / jobs.description (FR-10 context fields, §13): optional
    // show-name/description passed to llm-subtrans as --moviename/--description.
    for col in ["movie_name", "description"] {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM pragma_table_info('jobs') WHERE name = ?",
        )
        .bind(col)
        .fetch_one(pool)
        .await?;
        if count == 0 {
            let sql = format!("ALTER TABLE jobs ADD COLUMN {col} TEXT");
            sqlx::query(&sql).execute(pool).await?;
        }
    }
    Ok(())
}

/// Verify the database path is writable (boot validation, §6.12).
pub fn check_writable(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let probe = path.with_extension("write-test");
    std::fs::write(&probe, b"ok")?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}
