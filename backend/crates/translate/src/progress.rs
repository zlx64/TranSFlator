//! Parses llm-subtrans progress out of its log output (FR-13, D3).
//!
//! llm-subtrans logs (format `%(levelname)s: %(message)s`) lines like:
//! ```text
//! INFO: Translated batch 2.3: 90/240 lines (37%)
//! ```

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

static PROGRESS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"Translated batch \S+: (\d+)/(\d+) lines(?: \((\d+)%\))?").expect("valid regex")
});

/// A parsed progress event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub processed: u64,
    pub total: u64,
    pub pct: u8,
}

impl Progress {
    pub fn pct_f64(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            100.0 * self.processed as f64 / self.total as f64
        }
    }
}

/// Try to parse a progress event from a single log line. Returns `None` if the
/// line is not a progress line.
pub fn parse_progress_line(line: &str) -> Option<Progress> {
    let caps = PROGRESS_RE.captures(line)?;
    let processed: u64 = caps.get(1)?.as_str().parse().ok()?;
    let total: u64 = caps.get(2)?.as_str().parse().ok()?;
    let pct: u8 = if let Some(m) = caps.get(3) {
        m.as_str().parse().unwrap_or(0)
    } else if total > 0 {
        (100.0 * processed as f64 / total as f64) as u8
    } else {
        0
    };
    Some(Progress {
        processed,
        total,
        pct,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_progress_line() {
        let p = parse_progress_line("INFO: Translated batch 2.3: 90/240 lines (37%)").unwrap();
        assert_eq!(p.processed, 90);
        assert_eq!(p.total, 240);
        assert_eq!(p.pct, 37);
    }

    #[test]
    fn parses_without_percent() {
        let p = parse_progress_line("Translated batch 1.1: 10/40 lines").unwrap();
        assert_eq!(p.processed, 10);
        assert_eq!(p.total, 40);
        assert_eq!(p.pct, 25); // computed
    }

    #[test]
    fn ignores_non_progress_lines() {
        assert!(parse_progress_line("INFO: Initialising log").is_none());
        assert!(parse_progress_line("ERROR: something failed").is_none());
        assert!(parse_progress_line("").is_none());
        assert!(parse_progress_line("Loading subtitles...").is_none());
    }

    #[test]
    fn pct_f64_handles_zero_total() {
        let p = Progress {
            processed: 0,
            total: 0,
            pct: 0,
        };
        assert_eq!(p.pct_f64(), 0.0);
    }

    #[test]
    fn parses_full_line_with_tokens() {
        // Real verbose lines include token usage after the progress.
        let p = parse_progress_line(
            "INFO: Translated batch 5.2: 240/240 lines (100%) [1200 in / 300 out tokens]",
        )
        .unwrap();
        assert_eq!(p.pct, 100);
        assert_eq!(p.total, 240);
    }
}
