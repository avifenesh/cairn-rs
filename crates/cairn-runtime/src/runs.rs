//! Run service boundary per RFC 005.
//!
//! A run is a single execution attempt inside a session.
//! Runs are the primary execution unit for replay and runtime inspection.

use async_trait::async_trait;
use cairn_domain::commands::StartRun;
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
    ///
    /// Read-only projection lookup: no `session_id` parameter because the
    /// projection is keyed by `run_id` and carries `session_id` on the
    /// record.
    async fn get(&self, run_id: &RunId) -> Result<Option<RunRecord>, RuntimeError>;

    /// List runs in a session.
    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RunRecord>, RuntimeError>;

    /// Complete a run (terminal).
    ///
    /// `session_id` is required because the FF `ExecutionId` is minted
    /// from `(project, session_id, run_id)` via
    /// [`cairn_fabric::id_map::session_run_to_execution_id`]. Callers on
    /// the HTTP path fetch `session_id` from the cairn-store projection
    /// (`RunRecord::session_id`) before invoking this method; a mismatch
    /// means the FCALL targets a non-existent execution.
    async fn complete(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError>;

    /// Fail a run (terminal).
    async fn fail(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        failure_class: FailureClass,
    ) -> Result<RunRecord, RuntimeError>;

    /// Cancel a run (terminal).
    async fn cancel(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError>;

    /// Pause a run.
    async fn pause(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        reason: PauseReason,
    ) -> Result<RunRecord, RuntimeError>;

    /// Resume a paused run.
    async fn resume(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        trigger: ResumeTrigger,
        target: RunResumeTarget,
    ) -> Result<RunRecord, RuntimeError>;

    /// Claim the run so downstream suspend / signal operations can run.
    ///
    /// Unlike [`TaskService::claim`](crate::tasks::TaskService::claim),
    /// there is no `lease_owner` / `lease_duration_ms` parameter: runs
    /// are not worker-scheduled. Implementation details (worker identity,
    /// lease TTL, FCALL dispatch) live in the adapter.
    ///
    /// **NOT idempotent.** A second claim on an already-active run fails
    /// at the adapter's eligibility gate. Claim once per lifecycle; do
    /// not retry on success. A second claim is only legitimate after a
    /// suspend/resume cycle has made the run eligible again.
    async fn claim(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError>;

    /// Transition a run to WaitingApproval (approval gate).
    async fn enter_waiting_approval(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError>;

    /// Transition a run out of WaitingApproval after approval resolution.
    ///
    /// On approve: resumes to Running.
    /// On reject: fails with FailureClass::ApprovalRejected.
    async fn resolve_approval(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        decision: cairn_domain::ApprovalDecision,
    ) -> Result<RunRecord, RuntimeError>;

    /// Start a run from a [`StartRun`] command envelope (trigger path).
    ///
    /// Convenience unpacker around [`Self::start`]. Default impl delegates;
    /// specialised backends may override to skip field-by-field unpacking.
    async fn start_command(&self, command: StartRun) -> Result<RunRecord, RuntimeError> {
        self.start(
            &command.project,
            &command.session_id,
            command.run_id,
            command.parent_run_id,
        )
        .await
    }

    /// Start a run with an external correlation identifier (sqeq ingress).
    ///
    /// The correlation id is tagged on the emitted event so downstream
    /// consumers (observability, audit) can join back to the originating
    /// request. The default impl ignores the correlation id and delegates
    /// to [`Self::start`] — override when your backend carries correlation
    /// through to the event log.
    async fn start_with_correlation(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
        _correlation_id: &str,
    ) -> Result<RunRecord, RuntimeError> {
        self.start(project, session_id, run_id, parent_run_id).await
    }

    /// Spawn a subagent run linked to a parent.
    ///
    /// Subagent runs inherit the session and are tracked by the parent for
    /// hierarchical cancellation. Default impl constructs a child id from
    /// the parent if none supplied and calls [`Self::start`].
    async fn spawn_subagent(
        &self,
        project: &ProjectKey,
        parent_run_id: RunId,
        session_id: &SessionId,
        child_run_id: Option<RunId>,
    ) -> Result<RunRecord, RuntimeError> {
        let child = child_run_id
            .unwrap_or_else(|| RunId::new(format!("subagent_{}", parent_run_id.as_str())));
        self.start(project, session_id, child, Some(parent_run_id))
            .await
    }

    /// List child runs for a parent.
    ///
    /// Backed by the event-log projection in the in-memory impl; the Fabric
    /// adapter reads from the store projection (FF has no native
    /// parent-run index).
    async fn list_child_runs(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<RunRecord>, RuntimeError>;
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
