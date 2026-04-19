//! Task service boundary per RFC 005.
//!
//! Tasks are schedulable units of work with lease-based ownership.
//! Tasks support the full lifecycle: queued -> leased -> running -> terminal.

use async_trait::async_trait;
use cairn_domain::{
    FailureClass, PauseReason, ProjectKey, ResumeTrigger, RunId, SessionId, TaskId,
    TaskResumeTarget, TaskState,
};
use cairn_store::projections::TaskRecord;

use crate::error::RuntimeError;

/// Task service boundary.
///
/// Per RFC 005:
/// - tasks are the only leased execution entity in v1
/// - expired leases are recovered by the runtime
/// - retryable_failed is non-terminal and may return to queued
#[async_trait]
pub trait TaskService: Send + Sync {
    /// Submit a new task.
    ///
    /// `session_id` scopes the mint path: `Some(sid)` co-locates the task
    /// on the session's FlowId partition; `None` mints via the solo path
    /// for bare tasks (e.g. A2A). The choice at submit time MUST match
    /// every downstream mutation's `session_id` argument; a mismatch
    /// targets a non-existent FF execution.
    async fn submit(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: TaskId,
        parent_run_id: Option<RunId>,
        parent_task_id: Option<TaskId>,
        priority: u32,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Declare that `dependent_task_id` cannot start until `prerequisite_task_id` completes.
    ///
    /// Transitions `dependent_task_id` to `WaitingDependency`.
    async fn declare_dependency(
        &self,
        dependent_task_id: &TaskId,
        prerequisite_task_id: &TaskId,
    ) -> Result<cairn_store::projections::TaskDependencyRecord, RuntimeError>;

    /// Return unresolved (blocking) dependencies for `task_id`.
    ///
    /// If all dependencies are resolved, transitions the task to `Queued`.
    async fn check_dependencies(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<cairn_store::projections::TaskDependencyRecord>, RuntimeError>;

    /// Get a task by ID.
    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, RuntimeError>;

    /// Claim a task lease (queued -> leased).
    ///
    /// `session_id` must match the value supplied at [`Self::submit`]
    /// time — the FF `ExecutionId` is cached in no projection; it is
    /// re-derived on every call from `(project, session_id, task_id)`.
    /// Handler path fetches it from `TaskRecord` (via its
    /// `parent_run_id` → `RunRecord.session_id`) before invoking.
    async fn claim(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        lease_owner: String,
        lease_duration_ms: u64,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Heartbeat to extend a lease.
    async fn heartbeat(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        lease_extension_ms: u64,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Transition to running (leased -> running).
    async fn start(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Complete a task (terminal).
    async fn complete(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Fail a task (terminal or retryable).
    async fn fail(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        failure_class: FailureClass,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Cancel a task (terminal).
    async fn cancel(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Dead-letter a task (terminal, after exhausting retries).
    async fn dead_letter(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError>;

    /// RFC 005: query the dead-letter queue — tasks that exhausted all retry attempts.
    async fn list_dead_lettered(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError>;

    /// Pause a task.
    async fn pause(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        reason: PauseReason,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Resume a paused task.
    async fn resume(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        trigger: ResumeTrigger,
        target: TaskResumeTarget,
    ) -> Result<TaskRecord, RuntimeError>;

    /// List tasks by state (e.g., queued tasks for scheduling).
    async fn list_by_state(
        &self,
        project: &ProjectKey,
        state: TaskState,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError>;

    /// List tasks with expired leases (for recovery).
    async fn list_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError>;

    /// Release a task lease (leased -> queued), clearing lease_owner.
    async fn release_lease(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Spawn a subagent task linked to a parent run.
    ///
    /// Default impl submits a task with `parent_run_id = Some(parent_run_id)`
    /// and `priority = 0`. The in-memory impl overrides with an event-log
    /// path that emits `TaskCreated` + `SubagentSpawned` directly; the
    /// Fabric adapter inherits the default and routes through
    /// `FabricTaskService::submit` so FF gets the full flow.
    ///
    /// `child_session_id` / `child_run_id` are carried for the
    /// `SubagentSpawned` linkage. The default impl ignores them because
    /// the trait-level surface cannot emit that event without the
    /// underlying store; impls that need the linkage override this method.
    async fn spawn_subagent(
        &self,
        project: &ProjectKey,
        parent_run_id: RunId,
        _parent_task_id: Option<TaskId>,
        child_task_id: TaskId,
        child_session_id: SessionId,
        _child_run_id: Option<RunId>,
    ) -> Result<TaskRecord, RuntimeError> {
        // Subagent tasks are scoped to the parent's session so the
        // child execution co-locates on the session's FlowId partition
        // with the parent run.
        self.submit(
            project,
            Some(&child_session_id),
            child_task_id,
            Some(parent_run_id),
            None,
            0,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use cairn_domain::TaskState;

    #[test]
    fn retryable_failed_is_non_terminal() {
        assert!(TaskState::RetryableFailed.is_retryable());
        assert!(!TaskState::RetryableFailed.is_terminal());
    }
}
