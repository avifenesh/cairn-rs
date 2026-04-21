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
///
/// ## Session binding and RFC-011 co-location
///
/// Every method accepts `session_id: Option<&SessionId>` rather than a
/// required `&SessionId` because the A2A protocol submits session-less
/// tasks (see `/v1/a2a/tasks`). For all session-bound tasks, **callers
/// may pass `None`** and rely on the fabric adapter's
/// `resolve_task_project_and_session` to derive the session from the
/// projection on every mutation:
///
/// 1. `TaskRecord.session_id`, if present on the projection row, OR
/// 2. `TaskRecord.parent_run_id → RunRecord.session_id`, OR
/// 3. `None` (solo-mint path, A2A tasks).
///
/// The adapter is the single source of truth for partition placement.
/// HTTP handlers do **not** need to pre-resolve session — calling
/// the trait method with `None` is the contract.
///
/// One caller-side exception survives in `cairn-app`'s
/// `create_task_handler`: when submitting with `parent_task_id` but no
/// `parent_run_id`, the handler walks `parent_task_id →
/// RunRecord.session_id` because neither the adapter's `submit` nor
/// the `TaskCreated` projection writer follows that edge. The caller
/// therefore passes `Some(sid)` on that one submit path to preserve
/// co-location for sub-sub-tasks.
///
/// Session-less (bare / A2A) tasks route through the solo-mint path
/// and land on the same `PartitionFamily::Flow` keyspace but without
/// the session co-location guarantee. See
/// `docs/design/rfcs/RFC-011-flow-partitioning.md` for the full contract.
#[async_trait]
pub trait TaskService: Send + Sync {
    /// Submit a new task.
    ///
    /// `session_id` scopes the mint path: `Some(sid)` co-locates the task
    /// on the session's FlowId partition; `None` delegates resolution to
    /// the fabric adapter (see trait-level rustdoc for the derivation
    /// chain). A2A bare tasks submitted with neither `parent_run_id` nor
    /// `session_id` land on the solo partition.
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
    /// Under the Fabric backend this issues `ff_stage_dependency_edge` +
    /// `ff_apply_dependency_to_child` against FF. Both tasks must belong
    /// to the same session (FF flows are session-scoped).
    async fn declare_dependency(
        &self,
        dependent_task_id: &TaskId,
        prerequisite_task_id: &TaskId,
    ) -> Result<cairn_domain::TaskDependencyRecord, RuntimeError>;

    /// Return unresolved (blocking) dependencies for `task_id`.
    ///
    /// Under the Fabric backend this queries `ff_evaluate_flow_eligibility`
    /// and, when the task is blocked, reads the in-adjacency set to
    /// enumerate upstream task_ids. An empty return means no active
    /// blockers (eligible or already moved past the waiting state).
    async fn check_dependencies(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<cairn_domain::TaskDependencyRecord>, RuntimeError>;

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
