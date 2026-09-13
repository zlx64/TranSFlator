//! ffprobe result cache (FR-5).
//!
//! Results are keyed by file path + size + mtime. If any of those change the
//! cache entry is considered stale and re-probed.

use crate::model::MediaInfo;
use sqlx::Row;
use sqlx::SqlitePool;
use std::collections::HashMap;

/// Lightweight per-file metadata for the library listing (FR-2). Stream counts
/// are derived from the stored streams (they are not columns).
#[derive(Debug, Clone)]
pub struct CachedMeta {
    pub duration_s: Option<f64>,
    pub container: Option<String>,
    pub audio_count: u32,
    pub subtitle_count: u32,
}

/// SQLite-backed cache of ffprobe results.
pub struct MediaCache<'a> {
    pub pool: &'a SqlitePool,
}

impl<'a> MediaCache<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Return a cached [`MediaInfo`] if an entry matches `path`/`size`/`mtime`.
    pub async fn get(&self, path: &str, size: u64, mtime: i64) -> Option<MediaInfo> {
        let row = sqlx::query(
            "SELECT streams_json, duration_s, container \
              FROM media_cache WHERE path = ? AND size = ? AND mtime = ?",
        )
        .bind(path)
        .bind(size as i64)
        .bind(mtime)
        .fetch_optional(self.pool)
        .await
        .ok()?;
        let row = row?;

        let streams_json: String = row.try_get("streams_json").ok()?;
        let duration_s: Option<f64> = row.try_get("duration_s").ok()?;
        let container: Option<String> = row.try_get("container").ok()?;

        let streams: Vec<crate::model::Stream> = serde_json::from_str(&streams_json).ok()?;
        Some(rebuild(path, duration_s, container, size, streams))
    }

    /// Store a fresh ffprobe result.
    pub async fn put(&self, info: &MediaInfo, size: u64, mtime: i64) -> anyhow::Result<()> {
        let streams_json = serde_json::to_string(&info.streams)?;
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT OR REPLACE INTO media_cache \
             (path, size, mtime, duration_s, container, streams_json, scanned_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&info.path)
        .bind(size as i64)
        .bind(mtime)
        .bind(info.duration_s)
        .bind(&info.container)
        .bind(&streams_json)
        .bind(&now)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Batch lookup of cached metadata for many `(path, size, mtime)` keys.
    /// Returns a map from the (absolute) path to its metadata; missing or stale
    /// keys are omitted. Used to enrich library listings (FR-2) from the
    /// existing ffprobe cache without re-probing.
    pub async fn get_many(
        &self,
        keys: &[(String, u64, i64)],
    ) -> HashMap<String, CachedMeta> {
        let mut out = HashMap::new();
        for (path, size, mtime) in keys {
            if let Some(info) = self.get(path, *size, *mtime).await {
                let (_, audio_count, subtitle_count) =
                    crate::classify::count_kinds(&info.streams);
                out.insert(
                    path.clone(),
                    CachedMeta {
                        duration_s: info.duration_s,
                        container: info.container,
                        audio_count: audio_count as u32,
                        subtitle_count: subtitle_count as u32,
                    },
                );
            }
        }
        out
    }
}

fn rebuild(
    path: &str,
    duration_s: Option<f64>,
    container: Option<String>,
    size: u64,
    streams: Vec<crate::model::Stream>,
) -> MediaInfo {
    let (video_count, audio_count, subtitle_count) =
        crate::classify::count_kinds(&streams);
    MediaInfo {
        path: path.to_string(),
        duration_s,
        container,
        // Not persisted (spec §9 schema); cosmetic only.
        format_name: None,
        size: Some(size),
        streams,
        video_count,
        audio_count,
        subtitle_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Stream, StreamKind, SubtitleKind};

    fn sample_info() -> MediaInfo {
        MediaInfo {
            path: "/media/Show/E01.mkv".into(),
            duration_s: Some(3600.5),
            container: Some("matroska".into()),
            format_name: Some("matroska,webm".into()),
            size: Some(123),
            streams: vec![
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
                    kind: StreamKind::Subtitle,
                    codec: "subrip".into(),
                    language: Some("jpn".into()),
                    title: None,
                    default: true,
                    forced: false,
                    subtitle_kind: Some(SubtitleKind::Text),
                },
            ],
            video_count: 1,
            audio_count: 0,
            subtitle_count: 1,
        }
    }

    /// Pool against the real production schema (keeps the tempdir alive).
    async fn pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let p = transflator_config::db::init_pool(&dir.path().join("t.db"))
            .await
            .unwrap();
        (dir, p)
    }

    #[tokio::test]
    async fn put_then_get_roundtrip() {
        let (_dir, pool) = pool().await;
        let cache = MediaCache::new(&pool);
        let info = sample_info();
        cache.put(&info, 123, 1000).await.unwrap();

        let got = cache
            .get("/media/Show/E01.mkv", 123, 1000)
            .await
            .expect("expected cache hit");
        assert_eq!(got.streams.len(), 2);
        assert_eq!(got.streams[1].codec, "subrip");
        assert_eq!(got.streams[1].language.as_deref(), Some("jpn"));
        assert_eq!(got.duration_s, Some(3600.5));
        assert_eq!(got.container.as_deref(), Some("matroska"));
        assert_eq!(got.size, Some(123));
        assert_eq!(got.subtitle_count, 1);
    }

    #[tokio::test]
    async fn stale_key_misses() {
        let (_dir, pool) = pool().await;
        let cache = MediaCache::new(&pool);
        let info = sample_info();
        cache.put(&info, 123, 1000).await.unwrap();

        assert!(cache.get("/media/Show/E01.mkv", 124, 1000).await.is_none());
        assert!(cache.get("/media/Show/E01.mkv", 123, 1001).await.is_none());
        assert!(cache.get("/other/E01.mkv", 123, 1000).await.is_none());
    }
}
