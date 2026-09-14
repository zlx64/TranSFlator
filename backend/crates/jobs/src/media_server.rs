//! Best-effort Plex/Jellyfin notifications after a translation job completes.
//!
//! The media server is asked to refresh the movie or episode that matches the
//! job's source file. Failures are logged but never fail the translation job.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;
use sqlx::SqlitePool;
use transflator_config::secrets::Secrets;
use transflator_config::settings::{keys, SettingsStore};

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("TranSFlator/0.1")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

fn normalize_base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn normalize_path(path: &str) -> String {
    let mut out = path.trim().replace('\\', "/");
    while out.contains("//") {
        out = out.replace("//", "/");
    }
    out
}

async fn configured_pair(
    settings: &SettingsStore<'_>,
    url_key: &str,
    token_key: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let url = settings
        .get(url_key)
        .await?
        .filter(|s| !s.trim().is_empty());
    let token = settings
        .get_secret(token_key)
        .await?
        .filter(|s| !s.trim().is_empty());
    Ok(match (url, token) {
        (Some(url), Some(token)) => Some((normalize_base(&url), token.trim().to_string())),
        _ => None,
    })
}

async fn resolve_url(
    settings: &SettingsStore<'_>,
    key: &str,
    provided: Option<&str>,
) -> anyhow::Result<String> {
    let provided = provided.map(str::trim).filter(|s| !s.is_empty());
    let raw = if let Some(value) = provided {
        value.to_string()
    } else {
        settings
            .get(key)
            .await?
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("no server URL configured"))?
    };
    if !(raw.starts_with("http://") || raw.starts_with("https://")) {
        anyhow::bail!("server URL must start with http:// or https://");
    }
    Ok(normalize_base(&raw))
}

async fn resolve_token(
    settings: &SettingsStore<'_>,
    key: &str,
    provided: Option<&str>,
    label: &str,
) -> anyhow::Result<String> {
    let provided = provided.map(str::trim).filter(|s| !s.is_empty());
    let token = if let Some(value) = provided {
        value.to_string()
    } else {
        settings
            .get_secret(key)
            .await?
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("no {label} configured"))?
    };
    Ok(token.trim().to_string())
}

/// Test a Plex Media Server URL and token.
pub async fn test_plex(
    http: &reqwest::Client,
    settings: &SettingsStore<'_>,
    server_url: Option<&str>,
    token_override: Option<&str>,
) -> anyhow::Result<String> {
    let base = resolve_url(settings, keys::PLEX_SERVER_URL, server_url).await?;
    let token = resolve_token(settings, keys::PLEX_TOKEN, token_override, "Plex token").await?;
    test_plex_base(http, &base, &token).await
}

async fn test_plex_base(http: &reqwest::Client, base: &str, token: &str) -> anyhow::Result<String> {
    let identity = http
        .get(format!("{base}/identity"))
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = identity.status();
    if !status.is_success() {
        anyhow::bail!("Plex server returned {status}");
    }
    let value: Value = identity.json().await?;
    let version = value
        .get("MediaContainer")
        .and_then(|v| v.get("version"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    let sections = http
        .get(format!("{base}/library/sections"))
        .header("X-Plex-Token", token)
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = sections.status();
    if !status.is_success() {
        anyhow::bail!("Plex token was rejected by the server (status {status})");
    }

    if version.is_empty() {
        Ok("Connected to Plex".to_string())
    } else {
        Ok(format!("Connected to Plex {version}"))
    }
}

/// Test a Jellyfin server URL and API key.
pub async fn test_jellyfin(
    http: &reqwest::Client,
    settings: &SettingsStore<'_>,
    server_url: Option<&str>,
    token_override: Option<&str>,
) -> anyhow::Result<String> {
    let base = resolve_url(settings, keys::JELLYFIN_SERVER_URL, server_url).await?;
    let token = resolve_token(
        settings,
        keys::JELLYFIN_TOKEN,
        token_override,
        "Jellyfin API key",
    )
    .await?;
    test_jellyfin_base(http, &base, &token).await
}

async fn test_jellyfin_base(
    http: &reqwest::Client,
    base: &str,
    token: &str,
) -> anyhow::Result<String> {
    let resp = http
        .get(format!("{base}/system/info"))
        .header("X-Emby-Token", token)
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 401 || status.as_u16() == 403 {
            anyhow::bail!("Jellyfin API key was rejected by the server");
        }
        anyhow::bail!("Jellyfin server returned {status}");
    }
    let value: Value = resp.json().await?;
    let name = value
        .get("ServerName")
        .and_then(Value::as_str)
        .unwrap_or("Jellyfin");
    let version = value
        .get("Version")
        .and_then(Value::as_str)
        .unwrap_or_default();

    if version.is_empty() {
        Ok(format!("Connected to {name}"))
    } else {
        Ok(format!("Connected to {name} {version}"))
    }
}

/// Notify configured media servers that the source file's subtitle set changed.
pub async fn notify_job_done(pool: &SqlitePool, secrets: &Secrets, source_path: &Path) {
    let settings = SettingsStore::new(pool, secrets);
    let http = client();

    if let Err(e) = crate::plex_auth::ping_token_if_due(http, &settings).await {
        tracing::debug!(error = %e, "plex keep-alive ping setup failed");
    }

    match configured_pair(&settings, keys::PLEX_SERVER_URL, keys::PLEX_TOKEN).await {
        Ok(Some((base, token))) => {
            if let Err(e) = refresh_plex(http, &base, &token, source_path).await {
                tracing::warn!(error = %e, source = %source_path.display(), "plex refresh failed");
            }
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "failed to read plex settings"),
    }

    match configured_pair(&settings, keys::JELLYFIN_SERVER_URL, keys::JELLYFIN_TOKEN).await {
        Ok(Some((base, token))) => {
            if let Err(e) = refresh_jellyfin(http, &base, &token, source_path).await {
                tracing::warn!(
                    error = %e,
                    source = %source_path.display(),
                    "jellyfin refresh failed"
                );
            }
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "failed to read jellyfin settings"),
    }
}

async fn refresh_plex(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    source: &Path,
) -> anyhow::Result<()> {
    let sections_url = format!("{base}/library/sections");
    let sections = client
        .get(&sections_url)
        .header("X-Plex-Token", token)
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = sections.status();
    if !status.is_success() {
        anyhow::bail!("plex /library/sections returned {status}");
    }
    let value: Value = sections.json().await?;
    let directories = value
        .get("MediaContainer")
        .and_then(|v| v.get("Directory"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for dir in directories {
        let Some(key) = dir.get("key").and_then(|v| v.as_str()) else {
            continue;
        };
        let section_path = if key.starts_with('/') {
            key.to_string()
        } else {
            format!("/library/sections/{key}")
        };
        if let Some(rating_key) =
            find_plex_rating_key(client, base, token, &section_path, source).await?
        {
            let url = format!("{base}/library/metadata/{rating_key}/refresh");
            let resp = client
                .put(&url)
                .header("X-Plex-Token", token)
                .header("Accept", "application/json")
                .send()
                .await?;
            let status = resp.status();
            if status.is_success() {
                tracing::info!(
                    rating_key = %rating_key,
                    source = %source.display(),
                    "plex item refresh requested"
                );
                return Ok(());
            }
            anyhow::bail!("plex item refresh returned {status}");
        }
    }

    tracing::warn!(
        source = %source.display(),
        "plex item not found; no refresh sent"
    );
    Ok(())
}

async fn find_plex_rating_key(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    section_path: &str,
    source: &Path,
) -> anyhow::Result<Option<String>> {
    let target = normalize_path(&source.to_string_lossy());
    let filename = source
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());
    const PAGE_SIZE: usize = 1000;
    let mut start = 0usize;
    let mut filename_match: Option<String> = None;
    let mut ambiguous = false;

    for _ in 0..1000 {
        let url = format!(
            "{base}{section_path}/allLeaves?includeElements=Media&X-Plex-Container-Size={PAGE_SIZE}&X-Plex-Container-Start={start}"
        );
        let resp = client
            .get(&url)
            .header("X-Plex-Token", token)
            .header("Accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            tracing::debug!(
                section = %section_path,
                status = %status,
                "plex allLeaves query failed"
            );
            break;
        }
        let value: Value = resp.json().await?;
        let metadata = value
            .get("MediaContainer")
            .and_then(|v| v.get("Metadata"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if metadata.is_empty() {
            break;
        }

        for item in &metadata {
            let Some(rating_key) = item
                .get("ratingKey")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
            else {
                continue;
            };
            let Some(media_items) = item.get("Media").and_then(|v| v.as_array()) else {
                continue;
            };
            for media_item in media_items {
                let parts = media_item
                    .get("Part")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                for part in parts {
                    let Some(file) = part.get("file").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let candidate = normalize_path(file);
                    if candidate == target {
                        return Ok(Some(rating_key));
                    }
                    if let Some(name) = &filename {
                        if candidate.ends_with(&format!("/{}", name)) {
                            match &filename_match {
                                None => filename_match = Some(rating_key.clone()),
                                Some(prev) if prev == &rating_key => {}
                                Some(_) => {
                                    ambiguous = true;
                                    filename_match = None;
                                }
                            }
                        }
                    }
                }
            }
        }

        start += metadata.len();
        if metadata.len() < PAGE_SIZE {
            break;
        }
    }

    if ambiguous {
        return Ok(None);
    }
    Ok(filename_match)
}

async fn refresh_jellyfin(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    source: &Path,
) -> anyhow::Result<()> {
    let target = normalize_path(&source.to_string_lossy());
    let filename = source
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());
    const PAGE_SIZE: usize = 500;
    let mut start = 0usize;
    let mut filename_match: Option<String> = None;
    let mut ambiguous = false;

    for _ in 0..2000 {
        let url = format!(
            "{base}/Items?Recursive=true&IncludeItemTypes=Movie,Episode&Fields=Path&Limit={PAGE_SIZE}&StartIndex={start}&EnableTotalRecordCount=false"
        );
        let resp = client
            .get(&url)
            .header("X-Emby-Token", token)
            .header("Accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("jellyfin /Items returned {status}");
        }
        let value: Value = resp.json().await?;
        let items = value
            .get("Items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if items.is_empty() {
            break;
        }

        for item in &items {
            let Some(id) = item.get("Id").and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }) else {
                continue;
            };
            let Some(path) = item.get("Path").and_then(|v| v.as_str()) else {
                continue;
            };
            let candidate = normalize_path(path);
            if candidate == target {
                return refresh_jellyfin_item(client, base, token, &id).await;
            }
            if let Some(name) = &filename {
                if candidate.ends_with(&format!("/{}", name)) {
                    match &filename_match {
                        None => filename_match = Some(id),
                        Some(prev) if prev == &id => {}
                        Some(_) => {
                            ambiguous = true;
                            filename_match = None;
                        }
                    }
                }
            }
        }

        start += items.len();
        if items.len() < PAGE_SIZE {
            break;
        }
    }

    if ambiguous {
        tracing::warn!(
            source = %source.display(),
            "jellyfin filename match was ambiguous; no refresh sent"
        );
        return Ok(());
    }
    if let Some(id) = filename_match {
        return refresh_jellyfin_item(client, base, token, &id).await;
    }

    tracing::warn!(
        source = %source.display(),
        "jellyfin item not found; no refresh sent"
    );
    Ok(())
}

async fn refresh_jellyfin_item(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    id: &str,
) -> anyhow::Result<()> {
    let url = format!("{base}/Items/{id}/Refresh");
    let resp = client
        .post(&url)
        .header("X-Emby-Token", token)
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = resp.status();
    if status.is_success() {
        tracing::info!(item_id = %id, "jellyfin item refresh requested");
        Ok(())
    } else {
        anyhow::bail!("jellyfin item refresh returned {status}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::{get, post, put};
    use axum::Router;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use transflator_config::db;

    #[test]
    fn normalizes_windows_and_duplicate_slashes() {
        assert_eq!(
            normalize_path(r"\\server\media\show\episode.mkv"),
            "/server/media/show/episode.mkv"
        );
        assert_eq!(
            normalize_path("/media//show///ep.mkv"),
            "/media/show/ep.mkv"
        );
    }

    #[derive(Clone)]
    struct Calls {
        list: Arc<Mutex<Vec<String>>>,
    }

    impl Calls {
        fn new() -> Self {
            Self {
                list: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    async fn start_server(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    async fn plex_sections(State(calls): State<Calls>) -> axum::Json<Value> {
        calls
            .list
            .lock()
            .await
            .push("/library/sections".to_string());
        axum::Json(serde_json::json!({
            "MediaContainer": {
                "Directory": [
                    { "key": "/library/sections/1" }
                ]
            }
        }))
    }

    async fn plex_all_leaves(State(calls): State<Calls>) -> axum::Json<Value> {
        calls
            .list
            .lock()
            .await
            .push("/library/sections/1/allLeaves".to_string());
        axum::Json(serde_json::json!({
            "MediaContainer": {
                "Metadata": [
                    {
                        "ratingKey": "42",
                        "Media": [
                            {
                                "Part": [
                                    { "file": "/media/Show/Episode.mkv" }
                                ]
                            }
                        ]
                    }
                ]
            }
        }))
    }

    async fn plex_refresh(State(calls): State<Calls>) -> StatusCode {
        calls
            .list
            .lock()
            .await
            .push("/library/metadata/42/refresh".to_string());
        StatusCode::NO_CONTENT
    }

    fn plex_router(calls: Calls) -> Router {
        Router::new()
            .route("/library/sections", get(plex_sections))
            .route("/library/sections/1/allLeaves", get(plex_all_leaves))
            .route("/library/metadata/42/refresh", put(plex_refresh))
            .with_state(calls)
    }

    async fn jellyfin_items(State(calls): State<Calls>) -> axum::Json<Value> {
        calls.list.lock().await.push("/Items".to_string());
        axum::Json(serde_json::json!({
            "Items": [
                {
                    "Id": "abc",
                    "Path": "/media/Show/Episode.mkv"
                }
            ]
        }))
    }

    async fn jellyfin_refresh(State(calls): State<Calls>) -> StatusCode {
        calls
            .list
            .lock()
            .await
            .push("/Items/abc/Refresh".to_string());
        StatusCode::NO_CONTENT
    }

    fn jellyfin_router(calls: Calls) -> Router {
        Router::new()
            .route("/Items", get(jellyfin_items))
            .route("/Items/abc/Refresh", post(jellyfin_refresh))
            .with_state(calls)
    }

    #[tokio::test]
    async fn plex_refresh_uses_all_leaves_and_matching_file() {
        let calls = Calls::new();
        let (base, server) = start_server(plex_router(calls.clone())).await;

        refresh_plex(
            client(),
            &base,
            "token",
            Path::new("/media/Show/Episode.mkv"),
        )
        .await
        .unwrap();

        let recorded = calls.list.lock().await.clone();
        server.abort();
        assert_eq!(
            recorded,
            vec![
                "/library/sections",
                "/library/sections/1/allLeaves",
                "/library/metadata/42/refresh"
            ]
        );
    }

    #[tokio::test]
    async fn jellyfin_refresh_uses_items_and_matching_path() {
        let calls = Calls::new();
        let (base, server) = start_server(jellyfin_router(calls.clone())).await;

        refresh_jellyfin(
            client(),
            &base,
            "token",
            Path::new("/media/Show/Episode.mkv"),
        )
        .await
        .unwrap();

        let recorded = calls.list.lock().await.clone();
        server.abort();
        assert_eq!(recorded, vec!["/Items", "/Items/abc/Refresh"]);
    }

    #[tokio::test]
    async fn notify_job_done_calls_configured_servers() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::init_pool(&dir.path().join("t.db")).await.unwrap();
        let secrets = Secrets::from_app_secret("test-secret");
        let settings = SettingsStore::new(&pool, &secrets);

        let plex_calls = Calls::new();
        let (plex_base, plex_server) = start_server(plex_router(plex_calls.clone())).await;
        let jellyfin_calls = Calls::new();
        let (jellyfin_base, jellyfin_server) =
            start_server(jellyfin_router(jellyfin_calls.clone())).await;

        settings
            .set(keys::PLEX_SERVER_URL, &plex_base)
            .await
            .unwrap();
        settings
            .set_secret(keys::PLEX_TOKEN, "plex-token")
            .await
            .unwrap();
        settings
            .set(keys::JELLYFIN_SERVER_URL, &jellyfin_base)
            .await
            .unwrap();
        settings
            .set_secret(keys::JELLYFIN_TOKEN, "jellyfin-token")
            .await
            .unwrap();

        notify_job_done(&pool, &secrets, Path::new("/media/Show/Episode.mkv")).await;

        let plex_recorded = plex_calls.list.lock().await.clone();
        let jellyfin_recorded = jellyfin_calls.list.lock().await.clone();
        plex_server.abort();
        jellyfin_server.abort();

        assert!(plex_recorded
            .iter()
            .any(|c| c == "/library/sections/1/allLeaves"));
        assert!(plex_recorded
            .iter()
            .any(|c| c == "/library/metadata/42/refresh"));
        assert!(jellyfin_recorded.iter().any(|c| c == "/Items/abc/Refresh"));
    }

    async fn plex_identity(State(calls): State<Calls>) -> axum::Json<Value> {
        calls.list.lock().await.push("/identity".to_string());
        axum::Json(serde_json::json!({
            "MediaContainer": { "version": "1.40.2" }
        }))
    }

    async fn plex_test_sections(State(calls): State<Calls>) -> axum::Json<Value> {
        calls
            .list
            .lock()
            .await
            .push("/library/sections".to_string());
        axum::Json(serde_json::json!({
            "MediaContainer": { "Directory": [] }
        }))
    }

    fn plex_test_router(calls: Calls) -> Router {
        Router::new()
            .route("/identity", get(plex_identity))
            .route("/library/sections", get(plex_test_sections))
            .with_state(calls)
    }

    async fn jellyfin_system_info(State(calls): State<Calls>) -> axum::Json<Value> {
        calls.list.lock().await.push("/system/info".to_string());
        axum::Json(serde_json::json!({
            "ServerName": "Jellyfin",
            "Version": "10.9.0"
        }))
    }

    fn jellyfin_test_router(calls: Calls) -> Router {
        Router::new()
            .route("/system/info", get(jellyfin_system_info))
            .with_state(calls)
    }

    #[tokio::test]
    async fn plex_test_verifies_identity_and_token() {
        let calls = Calls::new();
        let (base, server) = start_server(plex_test_router(calls.clone())).await;

        let message = test_plex_base(client(), &base, "token").await.unwrap();

        server.abort();
        assert_eq!(message, "Connected to Plex 1.40.2");
        assert_eq!(
            calls.list.lock().await.as_slice(),
            &["/identity", "/library/sections"]
        );
    }

    #[tokio::test]
    async fn jellyfin_test_verifies_system_info() {
        let calls = Calls::new();
        let (base, server) = start_server(jellyfin_test_router(calls.clone())).await;

        let message = test_jellyfin_base(client(), &base, "key").await.unwrap();

        server.abort();
        assert_eq!(message, "Connected to Jellyfin 10.9.0");
        assert_eq!(calls.list.lock().await.as_slice(), &["/system/info"]);
    }

    #[tokio::test]
    async fn plex_test_requires_configured_values() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::init_pool(&dir.path().join("t.db")).await.unwrap();
        let secrets = Secrets::from_app_secret("test-secret");
        let settings = SettingsStore::new(&pool, &secrets);

        let err = test_plex(client(), &settings, None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no server URL configured"));
    }

    #[tokio::test]
    async fn jellyfin_test_requires_configured_values() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::init_pool(&dir.path().join("t.db")).await.unwrap();
        let secrets = Secrets::from_app_secret("test-secret");
        let settings = SettingsStore::new(&pool, &secrets);

        let err = test_jellyfin(client(), &settings, None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no server URL configured"));
    }
}
