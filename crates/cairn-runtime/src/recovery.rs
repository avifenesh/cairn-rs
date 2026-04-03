//! Recovery service boundary per RFC 005.
//!
//! Recovery handles expired task leases, interrupted runs, incomplete
//! checkpoints, and blocked-on-dependency states. Recovery produces
//! explicit runtime events and must be idempotent.

use async_trait::async_trait;
use cairn_domain::{RunId, TaskId};
use serde::{Deserialize, Serialize};

use crate::error::RuntimeError;

/// Outcome of a single recovery action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Task lease expired; requeued for retry.
    TaskRequeued { task_id: TaskId },
    /// Task lease expired; failed (max retries exceeded).
    TaskFailed { task_id: TaskId },
    /// Task dead-lettered after exhausting retries.
    TaskDeadLettered { task_id: TaskId },
    /// Run resumed from checkpoint after interruption.
    RunResumedFromCheckpoint { run_id: RunId },
    /// Run failed due to unrecoverable interruption.
    RunFailed { run_id: RunId },
    /// Dependency already completed; unblocked waiting entity.
    DependencyResolved { run_id: RunId },
}

/// Summary of a recovery sweep.
#[derive(Clone, Debug, Default)]
pub struct RecoverySummary {
    pub actions: Vec<RecoveryAction>,
    pub scanned: usize,
}

/// Recovery service boundary.
///
/// Per RFC 005:
/// - task recovery is task-centric
/// - run recovery uses the latest checkpoint
/// - recovery must be idempotent
/// - recovery must never require the sidecar to be the source of truth
#[async_trait]
pub trait RecoveryService: Send + Sync {
    /// Scan and recover tasks with expired leases.
    async fn recover_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<RecoverySummary, RuntimeError>;

    /// Scan and recover interrupted runs using checkpoints.
    async fn recover_interrupted_runs(&self, limit: usize)
        -> Result<RecoverySummary, RuntimeError>;

    /// Resolve blocked-on-dependency states where dependency already finished.
    async fn resolve_stale_dependencies(
        &self,
        limit: usize,
    ) -> Result<RecoverySummary, RuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::RecoveryAction;
    use cairn_domain::TaskId;

    #[test]
    fn recovery_actions_are_distinct() {
        let requeued = RecoveryAction::TaskRequeued {
            task_id: TaskId::new("t1"),
        };
        let failed = RecoveryAction::TaskFailed {
            task_id: TaskId::new("t1"),
        };
        assert_ne!(requeued, failed);
    }
}
