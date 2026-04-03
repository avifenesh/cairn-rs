//! Synchronous projections for correctness-critical current-state tables.
//!
//! Per RFC 002, synchronous projections are updated within the same
//! transaction as event-log persistence. They provide the read models
//! that the runtime needs to validate the next command.
//!
//! Each entity module defines:
//! - A record struct representing current state
//! - A read-model trait for querying current state
//!
//! The `SyncProjection` trait in this module defines the write-side
//! contract: applying stored events to update current state.

pub mod approval;
pub mod checkpoint;
pub mod eval_run;
pub mod ingest_job;
pub mod org;
pub mod prompt;
pub mod routing;
pub mod mailbox;
pub mod run;
pub mod session;
pub mod signal;
pub mod task;
pub mod tool_invocation;

pub use approval::*;
pub use checkpoint::*;
pub use eval_run::*;
pub use ingest_job::*;
pub use org::*;
pub use prompt::*;
pub use routing::*;
pub use mailbox::*;
pub use run::*;
pub use session::*;
pub use signal::*;
pub use task::*;
pub use tool_invocation::*;

use crate::error::StoreError;
use crate::event_log::StoredEvent;

/// Synchronous projection that updates within the event-append transaction.
///
/// Concrete implementations receive a stored event and update the
/// corresponding current-state table. The runtime calls `apply` within
/// the same database transaction that persists the event to the log.
///
/// All `full_history` entities (RFC 002) have a corresponding sync
/// projection: session, run, task, approval, checkpoint, mailbox,
/// tool invocation.
pub trait SyncProjection: Send + Sync {
    /// Apply a stored event to update projection state.
    ///
    /// Called within the event-append transaction. Implementations
    /// should match on the event payload and update the relevant
    /// current-state record, advancing its version.
    fn apply(&self, event: &StoredEvent) -> Result<(), StoreError>;
}
