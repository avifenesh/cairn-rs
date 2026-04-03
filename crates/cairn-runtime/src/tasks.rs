//! Task service boundary per RFC 005.
//!
//! Tasks are schedulable units of work with lease-based ownership.
//! Tasks support the full lifecycle: queued -> leased -> running -> terminal.

use async_trait::async_trait;
use cairn_domain::{
    FailureClass, PauseReason, ProjectKey, ResumeTrigger, RunId, TaskId, TaskResumeTarget,
    TaskState,
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
    async fn submit(
        &self,
        project: &ProjectKey,
        task_id: TaskId,
        parent_run_id: Option<RunId>,
        parent_task_id: Option<TaskId>,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Get a task by ID.
    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, RuntimeError>;

    /// Claim a task lease (queued -> leased).
    async fn claim(
        &self,
        task_id: &TaskId,
        lease_owner: String,
        lease_duration_ms: u64,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Heartbeat to extend a lease.
    async fn heartbeat(
        &self,
        task_id: &TaskId,
        lease_extension_ms: u64,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Transition to running (leased -> running).
    async fn start(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError>;

    /// Complete a task (terminal).
    async fn complete(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError>;

    /// Fail a task (terminal or retryable).
    async fn fail(
        &self,
        task_id: &TaskId,
        failure_class: FailureClass,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Cancel a task (terminal).
    async fn cancel(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError>;

    /// Dead-letter a task (terminal, after exhausting retries).
    async fn dead_letter(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError>;

    /// Pause a task.
    async fn pause(
        &self,
        task_id: &TaskId,
        reason: PauseReason,
    ) -> Result<TaskRecord, RuntimeError>;

    /// Resume a paused task.
    async fn resume(
        &self,
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
