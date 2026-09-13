//! Job state machine (FR-16, §6.11).
//!
//! ```text
//!                 ┌──────────┐  start   ┌──────────┐
//!   create ──────►│  queued  │─────────►│ running  │
//!                 └────┬─────┘          └────┬─────┘
//!                      │ cancel              │
//!                      ▼                     ├─► done
//!                 ┌──────────┐               ├─► failed
//!                 │ canceled │◄──────────────┤  (cancel)
//!                 └──────────┘               └─► interrupted (restart)
//!
//! Any terminal state may return to `queued` via retry (FR-18).
//! ```

use crate::model::JobStatus;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("invalid job transition {from} → {to}")]
    Invalid { from: JobStatus, to: JobStatus },
}

/// The complete set of legal transitions.
pub fn can_transition(from: JobStatus, to: JobStatus) -> bool {
    match (from, to) {
        (JobStatus::Queued, JobStatus::Running)
        | (JobStatus::Queued, JobStatus::Canceled) => true,
        (JobStatus::Running, JobStatus::Done)
        | (JobStatus::Running, JobStatus::Failed)
        | (JobStatus::Running, JobStatus::Canceled)
        | (JobStatus::Running, JobStatus::Interrupted) => true,
        // Retry: any terminal state back to queued (FR-18).
        (JobStatus::Done | JobStatus::Failed | JobStatus::Canceled | JobStatus::Interrupted, JobStatus::Queued) => {
            true
        }
        _ => false,
    }
}

/// Validate a transition, returning the target state.
pub fn transition(from: JobStatus, to: JobStatus) -> Result<JobStatus, TransitionError> {
    if can_transition(from, to) {
        Ok(to)
    } else {
        Err(TransitionError::Invalid { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use JobStatus::*;

    fn legal() -> Vec<(JobStatus, JobStatus)> {
        vec![
            (Queued, Running),
            (Queued, Canceled),
            (Running, Done),
            (Running, Failed),
            (Running, Canceled),
            (Running, Interrupted),
            (Done, Queued),
            (Failed, Queued),
            (Canceled, Queued),
            (Interrupted, Queued),
        ]
    }

    #[test]
    fn legal_transitions_accepted() {
        for (from, to) in legal() {
            assert!(can_transition(from, to), "{from} → {to} should be legal");
            assert_eq!(transition(from, to), Ok(to));
        }
    }

    #[test]
    fn illegal_transitions_rejected() {
        // Exhaustive: every (from, to) pair not in the legal list must fail.
        let all = [Queued, Running, Done, Failed, Canceled, Interrupted];
        let legal = legal();
        let mut checked = 0;
        for &from in &all {
            for &to in &all {
                if from == to {
                    continue;
                }
                if legal.contains(&(from, to)) {
                    continue;
                }
                checked += 1;
                assert!(!can_transition(from, to), "{from} → {to} must be illegal");
                assert_eq!(
                    transition(from, to),
                    Err(TransitionError::Invalid { from, to }),
                    "{from} → {to}"
                );
            }
        }
        // 6 states × 5 other states = 30 distinct pairs, minus the legal ones.
        assert_eq!(checked, 30 - legal.len(), "exhaustive check coverage");
    }

    #[test]
    fn terminal_states() {
        assert!(!Queued.is_terminal());
        assert!(!Running.is_terminal());
        assert!(Done.is_terminal());
        assert!(Failed.is_terminal());
        assert!(Canceled.is_terminal());
        assert!(Interrupted.is_terminal());
    }

    #[test]
    fn status_roundtrip() {
        for s in [Queued, Running, Done, Failed, Canceled, Interrupted] {
            assert_eq!(JobStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(JobStatus::parse("bogus"), None);
    }
}
