//! Shared access-token auth (NFR-7).
//!
//! When an effective `auth_token` is set (stored in Settings, else the `AUTH_TOKEN`
//! env var), every `/api/*` request must present it via the
//! `Authorization: Bearer <token>` header or a `?token=` query param (used by the
//! WebSocket upgrade). The served SPA (non-`/api` static files) and `/api/healthz`
//! stay open so the client can render its token prompt.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;
use transflator_config::settings::{keys, SettingsStore};

use crate::error::ApiError;
use crate::state::AppState;

pub async fn require_token(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let path = req.uri().path();
    // Protect only API routes; leave the SPA and health checks open.
    if !path.starts_with("/api/") || path == "/api/healthz" {
        return Ok(next.run(req).await);
    }

    let settings = SettingsStore::new(&state.pool, &state.secrets);
    let stored = settings
        .get_secret(keys::AUTH_TOKEN)
        .await
        .map_err(ApiError::db)?;
    let expected = stored
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| state.config.auth_token.clone())
        .trim()
        .to_string();
    if expected.is_empty() {
        return Ok(next.run(req).await);
    }

    match extract_token(&req) {
        Some(provided) if constant_time_eq(provided.as_bytes(), expected.as_bytes()) => {
            Ok(next.run(req).await)
        }
        _ => {
            tracing::warn!(path, "invalid or missing access token");
            Err(ApiError::unauthorized("invalid or missing access token"))
        }
    }
}

fn extract_token(req: &Request) -> Option<String> {
    if let Some(auth) = req.headers().get(header::AUTHORIZATION) {
        if let Ok(v) = auth.to_str() {
            if let Some(t) = v.strip_prefix("Bearer ") {
                let t = t.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    // WebSocket upgrades carry the token as a query param.
    let query = req.uri().query()?.to_string();
    for pair in query.split('&') {
        if let Some(v) = pair.strip_prefix("token=") {
            if !v.is_empty() {
                return Some(percent_decode(v));
            }
        }
    }
    None
}

/// Minimal percent-decoding for the token query param (no external dep).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 3 <= bytes.len() => {
                if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Constant-time byte comparison to avoid a timing oracle on the token.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decodes_token() {
        assert_eq!(percent_decode("abc"), "abc");
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("%2F%2F"), "//");
        assert_eq!(percent_decode("bad%zz"), "bad%zz");
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secrex"));
        assert!(!constant_time_eq(b"secret", b"secret!"));
    }
}
