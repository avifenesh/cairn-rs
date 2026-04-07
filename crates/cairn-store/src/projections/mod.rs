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
pub mod approval_policy;
pub mod audit;
pub mod channel;
pub mod checkpoint;
pub mod checkpoint_strategy;
pub mod commercial;
pub mod credential;
pub mod defaults;
pub mod eval_baseline;
pub mod eval_dataset;
pub mod eval_rubric;
pub mod eval_run;
pub mod external_worker;
pub mod guardrail;
pub mod ingest_job;
pub mod llm_traces;
pub mod mailbox;
pub mod model_comparison;
pub mod notification;
pub mod operator_intervention;
pub mod org;
pub mod pause_schedule;
pub mod prompt;
pub mod provider;
pub mod quota;
pub mod recovery;
pub mod retention;
pub mod routing;
pub mod run;
pub mod run_sla;
pub mod session;
pub mod sharing;
pub mod scheduled_task;
pub mod signal;
pub mod snapshot;
pub mod task;
pub mod task_dependency;
pub mod operator_profile;
pub mod outcome;
pub mod signal_subscription;
pub mod workspace_membership;
pub mod tool_invocation;

pub use approval::*;
pub use approval_policy::*;
pub use audit::*;
pub use channel::*;
pub use checkpoint::*;
pub use checkpoint_strategy::*;
pub use commercial::*;
pub use credential::*;
pub use defaults::*;
pub use eval_baseline::*;
pub use eval_dataset::*;
pub use eval_rubric::*;
pub use eval_run::*;
pub use external_worker::*;
pub use guardrail::*;
pub use ingest_job::*;
pub use llm_traces::LlmCallTraceReadModel;
pub use mailbox::*;
pub use model_comparison::*;
pub use notification::*;
pub use operator_intervention::*;
pub use org::*;
pub use pause_schedule::*;
pub use prompt::*;
pub use provider::*;
pub use quota::*;
pub use recovery::*;
pub use retention::*;
pub use routing::*;
pub use run::*;
pub use run_sla::*;
pub use session::*;
pub use sharing::*;
pub use scheduled_task::ScheduledTaskReadModel;
pub use signal::*;
pub use snapshot::*;
pub use task::*;
pub use task_dependency::{TaskDependencyReadModel, TaskDependencyRecord};
pub use operator_profile::{OperatorProfileReadModel, OperatorProfileRecord};
pub use signal_subscription::{SignalSubscriptionReadModel, SignalSubscriptionRecord};
pub use workspace_membership::{WorkspaceMemberRecord, WorkspaceMembershipReadModel};
pub use tool_invocation::*;
pub use outcome::*;

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
