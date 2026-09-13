//! Job state machine, SQLite persistence, bounded-concurrency queue, and the
//! probe → extract → translate pipeline for TranSFlator (Phases 4.4–4.8).
//!
//! The manager is transport-agnostic: it owns the queue and emits
//! [`JobEventEnvelope`]s on a broadcast channel that the API layer bridges to
//! WebSockets (Phase 5).

pub mod events;
pub mod machine;
pub mod manager;
pub mod model;
pub mod runner;
pub mod store;

pub use events::{JobEvent, JobEventEnvelope};
pub use machine::{can_transition, transition, TransitionError};
pub use manager::{JobManager, ManagerError, ManagerResult};
pub use model::{Job, JobStatus, NewJob, RetryMode};
pub use runner::{FakeRunner, JobRunner, PipelineRunner, RunError};
pub use store::{JobStore, StoreError, LOG_TAIL_MAX};
