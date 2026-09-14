//! Plex TV PIN authentication, server discovery, and token keep-alive.
//!
//! This follows the same practical approach Sonarr uses: a stable client
//! identifier, a Plex TV PIN flow to obtain an `X-Plex-Token`, and the Plex TV
//! resource endpoint to discover owned Plex Media Server connections.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use transflator_config::settings::{keys, SettingsStore};

const PLEX_TV_BASE: &str = "https://plex.tv";
const PRODUCT: &str = "TranSFlator";
const PLATFORM: &str = "Web";
const PLATFORM_VERSION: &str = "1";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const PING_INTERVAL_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize)]
pub struct PlexPinStart {
    pub pin_id: u64,
    pub code: String,
    pub auth_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlexServerOption {
    pub name: String,
    pub url: String,
    pub host: String,
    pub port: u16,
    pub secure: bool,
    pub local: bool,
    pub hints: Vec<String>,
}

fn plex_headers(client_id: &str) -> Vec<(&'static str, String)> {
    vec![
        ("X-Plex-Client-Identifier", client_id.to_string()),
        ("X-Plex-Product", PRODUCT.to_string()),
        ("X-Plex-Platform", PLATFORM.to_string()),
        ("X-Plex-Platform-Version", PLATFORM_VERSION.to_string()),
        ("X-Plex-Device-Name", PRODUCT.to_string()),
        ("X-Plex-Version", VERSION.to_string()),
    ]
}

async fn ensure_client_identifier(settings: &SettingsStore<'_>) -> anyhow::Result<String> {
    if let Some(id) = settings
        .get(keys::PLEX_CLIENT_IDENTIFIER)
        .await?
        .filter(|s| !s.trim().is_empty())
    {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    settings.set(keys::PLEX_CLIENT_IDENTIFIER, &id).await?;
    Ok(id)
}

fn auth_url(client_id: &str, code: &str) -> String {
    format!(
        "https://app.plex.tv/auth#?clientID={client_id}&code={code}\
         &context%5Bdevice%5D%5Bproduct%5D={PRODUCT}\
         &context%5Bdevice%5D%5Bplatform%5D={PLATFORM}\
         &context%5Bdevice%5D%5BplatformVersion%5D={PLATFORM_VERSION}\
         &context%5Bdevice%5D%5Bversion%5D={VERSION}"
    )
}

fn is_authenticated(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        Some(Value::String(s)) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn host_from_url(url: &str) -> String {
    let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = without_scheme.split('/').next().unwrap_or("");
    authority
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(authority)
        .to_string()
}

pub async fn start_pin(
    http: &reqwest::Client,
    settings: &SettingsStore<'_>,
) -> anyhow::Result<PlexPinStart> {
    start_pin_base(http, settings, PLEX_TV_BASE).await
}

async fn start_pin_base(
    http: &reqwest::Client,
    settings: &SettingsStore<'_>,
    base: &str,
) -> anyhow::Result<PlexPinStart> {
    let client_id = ensure_client_identifier(settings).await?;
    let url = format!("{base}/api/v2/pins?strong=1");
    let mut request = http.post(&url);
    for (name, value) in plex_headers(&client_id) {
        request = request.header(name, value);
    }

    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("plex.tv PIN request returned {status}");
    }
    let value: Value = response.json().await?;
    let pin = value.get("PIN").unwrap_or(&value);
    let pin_id = pin
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("Plex PIN response is missing id"))?;
    let code = pin
        .get("code")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Plex PIN response is missing code"))?
        .to_string();
    let auth_url = auth_url(&client_id, &code);

    Ok(PlexPinStart {
        pin_id,
        code,
        auth_url,
    })
}

/// Poll a Plex PIN. Returns the auth token once the user has approved it.
pub async fn poll_pin(
    http: &reqwest::Client,
    settings: &SettingsStore<'_>,
    pin_id: u64,
) -> anyhow::Result<Option<String>> {
    poll_pin_base(http, settings, pin_id, PLEX_TV_BASE).await
}

async fn poll_pin_base(
    http: &reqwest::Client,
    settings: &SettingsStore<'_>,
    pin_id: u64,
    base: &str,
) -> anyhow::Result<Option<String>> {
    let client_id = ensure_client_identifier(settings).await?;
    let url = format!("{base}/api/v2/pins/{pin_id}");
    let mut request = http.get(&url);
    for (name, value) in plex_headers(&client_id) {
        request = request.header(name, value);
    }

    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("plex.tv PIN poll returned {status}");
    }
    let value: Value = response.json().await?;
    let pin = value.get("PIN").unwrap_or(&value);
    if !is_authenticated(pin.get("authenticated")) {
        return Ok(None);
    }

    let token = pin
        .get("authToken")
        .or_else(|| pin.get("accessToken"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());
    Ok(token)
}

pub async fn list_servers(
    http: &reqwest::Client,
    settings: &SettingsStore<'_>,
) -> anyhow::Result<Vec<PlexServerOption>> {
    list_servers_base(http, settings, PLEX_TV_BASE).await
}

async fn list_servers_base(
    http: &reqwest::Client,
    settings: &SettingsStore<'_>,
    base: &str,
) -> anyhow::Result<Vec<PlexServerOption>> {
    let token = settings
        .get_secret(keys::PLEX_TOKEN)
        .await?
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Plex token is not set"))?;
    let client_id = ensure_client_identifier(settings).await?;
    let url = format!("{base}/api/v2/resources?includeHttps=1");

    let mut request = http.get(&url).header("X-Plex-Token", token.trim());
    for (name, value) in plex_headers(&client_id) {
        request = request.header(name, value);
    }

    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("plex.tv resource discovery returned {status}");
    }
    let value: Value = response.json().await?;
    Ok(parse_servers(&value))
}

fn parse_servers(value: &Value) -> Vec<PlexServerOption> {
    let resources = value.as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for resource in resources {
        if !resource
            .get("Owned")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let provides = resource
            .get("Provides")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !provides
            .split(',')
            .any(|p| p.trim().eq_ignore_ascii_case("server"))
        {
            continue;
        }

        let name = resource
            .get("Name")
            .and_then(Value::as_str)
            .unwrap_or("Plex Media Server")
            .to_string();
        let fallback_address = resource
            .get("Address")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let fallback_port = resource
            .get("Port")
            .and_then(Value::as_u64)
            .map(|p| p as u16)
            .unwrap_or(32400);
        let connections = resource
            .get("Connections")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if connections.is_empty() {
            if let Some(address) = fallback_address {
                let url = format!("http://{address}:{fallback_port}");
                if seen.insert(url.clone()) {
                    out.push(PlexServerOption {
                        name: format!("{name} ({address})"),
                        url,
                        host: address.clone(),
                        port: fallback_port,
                        secure: false,
                        local: false,
                        hints: vec!["Remote".to_string()],
                    });
                }
            }
            continue;
        }

        for connection in connections {
            let uri = connection
                .get("URI")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string());
            let address = connection
                .get("Address")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
                .or_else(|| fallback_address.clone());
            let port = connection
                .get("Port")
                .and_then(Value::as_u64)
                .map(|p| p as u16)
                .unwrap_or(fallback_port);
            let local = connection
                .get("Local")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let (url, host, secure) = if let Some(url) = uri {
                let secure = url.starts_with("https://");
                (url.clone(), host_from_url(&url), secure)
            } else if let Some(address) = address {
                (format!("http://{address}:{port}"), address.clone(), false)
            } else {
                continue;
            };

            if !seen.insert(url.clone()) {
                continue;
            }

            let mut hints = vec![if local {
                "Local".to_string()
            } else {
                "Remote".to_string()
            }];
            if secure {
                hints.push("Secure".to_string());
            }

            out.push(PlexServerOption {
                name: format!("{name} ({host})"),
                url,
                host,
                port,
                secure,
                local,
                hints,
            });
        }
    }

    out
}

/// Best-effort Plex TV keep-alive ping, at most once per 24 hours.
pub async fn ping_token_if_due(
    http: &reqwest::Client,
    settings: &SettingsStore<'_>,
) -> anyhow::Result<()> {
    ping_token_if_due_base(http, settings, PLEX_TV_BASE).await
}

async fn ping_token_if_due_base(
    http: &reqwest::Client,
    settings: &SettingsStore<'_>,
    base: &str,
) -> anyhow::Result<()> {
    let Some(token) = settings
        .get_secret(keys::PLEX_TOKEN)
        .await?
        .filter(|s| !s.trim().is_empty())
    else {
        return Ok(());
    };

    if let Some(last) = settings.get(keys::PLEX_LAST_PING).await? {
        if let Ok(dt) = DateTime::parse_from_rfc3339(&last) {
            let last_utc = dt.with_timezone(&Utc);
            if (Utc::now() - last_utc).num_seconds() < PING_INTERVAL_SECS {
                return Ok(());
            }
        }
    }

    let client_id = ensure_client_identifier(settings).await?;
    let url = format!("{base}/api/v2/ping");
    let mut request = http.get(&url).header("X-Plex-Token", token.trim());
    for (name, value) in plex_headers(&client_id) {
        request = request.header(name, value);
    }

    let result = async {
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("plex.tv ping returned {status}");
        }
        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            settings.set(keys::PLEX_LAST_PING, &now).await?;
        }
        Err(e) => tracing::debug!(error = %e, "plex.tv ping failed"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::routing::{get, post};
    use axum::Router;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use transflator_config::db;
    use transflator_config::secrets::Secrets;

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

    async fn pins(State(calls): State<Calls>) -> axum::Json<Value> {
        calls.list.lock().await.push("/api/v2/pins".to_string());
        axum::Json(serde_json::json!({
            "PIN": { "id": 123, "code": "abc" }
        }))
    }

    async fn pin(State(calls): State<Calls>) -> axum::Json<Value> {
        calls.list.lock().await.push("/api/v2/pins/123".to_string());
        axum::Json(serde_json::json!({
            "PIN": {
                "id": 123,
                "code": "abc",
                "authenticated": 1,
                "authToken": "plex-token"
            }
        }))
    }

    async fn resources(State(calls): State<Calls>) -> axum::Json<Value> {
        calls
            .list
            .lock()
            .await
            .push("/api/v2/resources".to_string());
        axum::Json(serde_json::json!([
            {
                "Name": "Plex Media Server",
                "Owned": true,
                "Provides": "server,player",
                "Address": "127.0.0.1",
                "Port": 32400,
                "Connections": [
                    {
                        "Protocol": "tcp",
                        "Address": "127.0.0.1",
                        "Port": 32400,
                        "Local": true,
                        "URI": "http://127.0.0.1:32400"
                    }
                ]
            },
            {
                "Name": "Not Owned",
                "Owned": false,
                "Provides": "server",
                "Connections": []
            }
        ]))
    }

    async fn ping(State(calls): State<Calls>) -> axum::Json<Value> {
        calls.list.lock().await.push("/api/v2/ping".to_string());
        axum::Json(serde_json::json!({}))
    }

    fn router(calls: Calls) -> Router {
        Router::new()
            .route("/api/v2/pins", post(pins))
            .route("/api/v2/pins/123", get(pin))
            .route("/api/v2/resources", get(resources))
            .route("/api/v2/ping", get(ping))
            .with_state(calls)
    }

    async fn store() -> (sqlx::SqlitePool, Secrets, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::init_pool(&dir.path().join("plex.db")).await.unwrap();
        let secrets = Secrets::from_app_secret("test-secret");
        (pool, secrets, dir)
    }

    #[tokio::test]
    async fn start_pin_returns_auth_url_and_stores_client_id() {
        let (pool, secrets, _dir) = store().await;
        let settings = SettingsStore::new(&pool, &secrets);
        let calls = Calls::new();
        let (base, server) = start_server(router(calls.clone())).await;
        let http = reqwest::Client::new();

        let start = start_pin_base(&http, &settings, &base).await.unwrap();

        server.abort();
        let client_id = settings
            .get(keys::PLEX_CLIENT_IDENTIFIER)
            .await
            .unwrap()
            .unwrap();
        let recorded = calls.list.lock().await.clone();

        assert_eq!(start.pin_id, 123);
        assert_eq!(start.code, "abc");
        assert!(start.auth_url.contains(&client_id));
        assert!(start.auth_url.contains("code=abc"));
        assert_eq!(recorded, vec!["/api/v2/pins"]);
    }

    #[tokio::test]
    async fn poll_pin_returns_token_when_authenticated() {
        let (pool, secrets, _dir) = store().await;
        let settings = SettingsStore::new(&pool, &secrets);
        let calls = Calls::new();
        let (base, server) = start_server(router(calls.clone())).await;
        let http = reqwest::Client::new();

        let token = poll_pin_base(&http, &settings, 123, &base).await.unwrap();

        server.abort();
        assert_eq!(token.as_deref(), Some("plex-token"));
    }

    #[tokio::test]
    async fn list_servers_maps_owned_server_resources() {
        let (pool, secrets, _dir) = store().await;
        let settings = SettingsStore::new(&pool, &secrets);
        settings
            .set_secret(keys::PLEX_TOKEN, "token")
            .await
            .unwrap();
        let calls = Calls::new();
        let (base, server) = start_server(router(calls.clone())).await;
        let http = reqwest::Client::new();

        let servers = list_servers_base(&http, &settings, &base).await.unwrap();

        server.abort();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].url, "http://127.0.0.1:32400");
        assert_eq!(servers[0].host, "127.0.0.1");
        assert_eq!(servers[0].port, 32400);
        assert!(!servers[0].secure);
        assert!(servers[0].local);
        assert_eq!(servers[0].hints, vec!["Local"]);
    }

    #[tokio::test]
    async fn ping_token_if_due_records_timestamp() {
        let (pool, secrets, _dir) = store().await;
        let settings = SettingsStore::new(&pool, &secrets);
        settings
            .set_secret(keys::PLEX_TOKEN, "token")
            .await
            .unwrap();
        let calls = Calls::new();
        let (base, server) = start_server(router(calls.clone())).await;
        let http = reqwest::Client::new();

        ping_token_if_due_base(&http, &settings, &base)
            .await
            .unwrap();

        server.abort();
        let last = settings.get(keys::PLEX_LAST_PING).await.unwrap().unwrap();
        let recorded = calls.list.lock().await.clone();
        assert!(!last.is_empty());
        assert_eq!(recorded, vec!["/api/v2/ping"]);
    }
}
