//! Run service boundary per RFC 005.
//!
//! A run is a single execution attempt inside a session.
//! Runs are the primary execution unit for replay and runtime inspection.

use async_trait::async_trait;
use cairn_domain::{
    FailureClass, PauseReason, ProjectKey, ResumeTrigger, RunId, RunResumeTarget, SessionId,
};
use cairn_store::projections::RunRecord;

use crate::error::RuntimeError;

/// Run service boundary.
///
/// Per RFC 005:
/// - runs belong to one session
/// - runs may have parent_run_id for subagent linkage
/// - completed, failed, canceled are terminal
#[async_trait]
pub trait RunService: Send + Sync {
    /// Start a new run in a session.
    async fn start(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
    ) -> Result<RunRecord, RuntimeError>;

    /// Get a run by ID.
    async fn get(&self, run_id: &RunId) -> Result<Option<RunRecord>, RuntimeError>;

    /// List runs in a session.
    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RunRecord>, RuntimeError>;

    /// Complete a run (terminal).
    async fn complete(&self, run_id: &RunId) -> Result<RunRecord, RuntimeError>;

    /// Fail a run (terminal).
    async fn fail(
        &self,
        run_id: &RunId,
        failure_class: FailureClass,
    ) -> Result<RunRecord, RuntimeError>;

    /// Cancel a run (terminal).
    async fn cancel(&self, run_id: &RunId) -> Result<RunRecord, RuntimeError>;

    /// Pause a run.
    async fn pause(&self, run_id: &RunId, reason: PauseReason) -> Result<RunRecord, RuntimeError>;

    /// Resume a paused run.
    async fn resume(
        &self,
        run_id: &RunId,
        trigger: ResumeTrigger,
        target: RunResumeTarget,
    ) -> Result<RunRecord, RuntimeError>;
}

#[cfg(test)]
mod tests {
    use cairn_domain::RunState;

    #[test]
    fn terminal_run_states_match_rfc() {
        assert!(RunState::Completed.is_terminal());
        assert!(RunState::Failed.is_terminal());
        assert!(RunState::Canceled.is_terminal());
        assert!(!RunState::Running.is_terminal());
        assert!(!RunState::Paused.is_terminal());
    }
}
