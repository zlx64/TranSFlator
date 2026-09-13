//! Classification of a failed llm-subtrans run (§6.6).
//!
//! The runner uses this to decide what to do when the subprocess exits
//! non-zero:
//! - [`FailureClass::Transient`] — rate limits / 429s, network timeouts,
//!   provider outages. Retry with backoff; llm-subtrans's project file resumes
//!   in place, so we don't lose progress.
//! - [`FailureClass::ContentSafety`] — a provider content-safety refusal (a
//!   documented Gemini behavior for certain lines). Never retry (it would just
//!   re-trigger); fail with a message that identifies the refusal, with the
//!   offending region left in the job log.
//! - [`FailureClass::Fatal`] — anything else (bad key, invalid model, ...).
//!   Fail fast, no retry.
//!
//! This is a pure function over the exit code + stderr tail so it can be
//! exhaustively unit-tested without spawning anything.

/// How a failed run should be treated by the runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Transient,
    ContentSafety,
    Fatal,
}

impl FailureClass {
    /// A short, user-presentable reason prefix for the failure.
    pub fn label(self) -> &'static str {
        match self {
            FailureClass::Transient => "transient provider error",
            FailureClass::ContentSafety => "provider content-safety refusal",
            FailureClass::Fatal => "translation failed",
        }
    }
}

/// Classify a failed run from its exit code and (lowercased-friendly) stderr.
pub fn classify_failure(exit_code: i32, stderr: &str) -> FailureClass {
    let text = stderr.to_ascii_lowercase();
    // Content-safety refusals are specific — check first so a refusal that also
    // mentions "retry" is never retried (retrying just re-triggers it).
    if content_safety(&text) {
        return FailureClass::ContentSafety;
    }
    if transient(&text, exit_code) {
        return FailureClass::Transient;
    }
    FailureClass::Fatal
}

fn content_safety(t: &str) -> bool {
    const MARKERS: &[&str] = &[
        "content policy",
        "content_policy",
        "content filtering",
        "safety",
        "refus",
        "inappropriate",
        "blocked by",
        "flagged",
        "prohibited",
        "harmful",
        "violat",
    ];
    MARKERS.iter().any(|m| t.contains(m))
}

fn transient(t: &str, exit_code: i32) -> bool {
    // Some wrappers surface the HTTP status as the process exit code.
    if matches!(exit_code, 429 | 502 | 503 | 504) {
        return true;
    }
    const MARKERS: &[&str] = &[
        "rate limit",
        "rate-limit",
        "ratelimit",
        "too many requests",
        "timeout",
        "timed out",
        "deadline",
        "connection reset",
        "connection refused",
        "connection aborted",
        "network",
        "unreachable",
        "overloaded",
        "server is busy",
        "temporarily unavailable",
        "service unavailable",
        "bad gateway",
        "gateway timeout",
        "econnreset",
        "eai_again",
    ];
    MARKERS.iter().any(|m| t.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_status_exit_codes_are_transient() {
        assert_eq!(classify_failure(429, ""), FailureClass::Transient);
        assert_eq!(classify_failure(503, ""), FailureClass::Transient);
        assert_eq!(classify_failure(502, ""), FailureClass::Transient);
        assert_eq!(classify_failure(504, ""), FailureClass::Transient);
    }

    #[test]
    fn rate_limit_text_is_transient() {
        assert_eq!(
            classify_failure(1, "Error: rate limit exceeded, please retry later"),
            FailureClass::Transient
        );
        assert_eq!(
            classify_failure(1, "429 Too Many Requests from provider"),
            FailureClass::Transient
        );
    }

    #[test]
    fn timeout_and_network_are_transient() {
        assert_eq!(
            classify_failure(1, "Request timed out after 30s"),
            FailureClass::Transient
        );
        assert_eq!(
            classify_failure(1, "connection reset by peer"),
            FailureClass::Transient
        );
        assert_eq!(
            classify_failure(1, "503 Service Unavailable"),
            FailureClass::Transient
        );
    }

    #[test]
    fn content_safety_refusal_detected() {
        assert_eq!(
            classify_failure(
                1,
                "API error: content policy violation — the request was blocked"
            ),
            FailureClass::ContentSafety
        );
        assert_eq!(
            classify_failure(1, "The model refused to translate this line (safety)"),
            FailureClass::ContentSafety
        );
        assert_eq!(
            classify_failure(1, "inappropriate content flagged by provider"),
            FailureClass::ContentSafety
        );
    }

    #[test]
    fn content_safety_takes_precedence_over_transient() {
        // A refusal that also says "retry" must not be retried.
        assert_eq!(
            classify_failure(1, "content policy violation, please retry with fewer lines"),
            FailureClass::ContentSafety
        );
    }

    #[test]
    fn bad_key_and_unknown_model_are_fatal() {
        assert_eq!(
            classify_failure(1, "401 Unauthorized: invalid API key"),
            FailureClass::Fatal
        );
        assert_eq!(
            classify_failure(1, "unknown model: gpt-9-turbo"),
            FailureClass::Fatal
        );
        assert_eq!(classify_failure(1, ""), FailureClass::Fatal);
    }
}
