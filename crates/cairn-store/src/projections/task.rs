use async_trait::async_trait;
use cairn_domain::{
    FailureClass, PauseReason, ProjectKey, PromptReleaseId, ResumeTrigger, RunId, SessionId,
    TaskId, TaskState,
};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Current-state record for a task.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: TaskId,
    pub project: ProjectKey,
    pub parent_run_id: Option<RunId>,
    pub parent_task_id: Option<TaskId>,
    /// Session the task is scoped to. Populated from `TaskCreated.session_id`
    /// on new submissions (RFC-011 Phase 3). `None` for solo (session-less)
    /// tasks, and also for legacy tasks created before Phase 3 whose event
    /// carried no session_id — resolvers must still fall back to walking
    /// `parent_run_id → RunRecord.session_id` in that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    pub state: TaskState,
    pub prompt_release_id: Option<PromptReleaseId>,
    pub failure_class: Option<FailureClass>,
    pub pause_reason: Option<PauseReason>,
    pub resume_trigger: Option<ResumeTrigger>,
    pub retry_count: u32,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<u64>,
    /// Product-level title for operator/SSE surfaces.
    pub title: Option<String>,
    /// Product-level description for operator/SSE surfaces.
    pub description: Option<String>,
    pub version: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Read-model for task current state.
#[async_trait]
pub trait TaskReadModel: Send + Sync {
    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, StoreError>;

    /// List tasks in a given state for a project (e.g., queued tasks for scheduling).
    async fn list_by_state(
        &self,
        project: &ProjectKey,
        state: TaskState,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError>;

    /// List tasks with expired leases (for recovery sweeps).
    async fn list_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError>;

    /// List child tasks spawned by a parent run (for stale-dependency resolution).
    /// Returns tasks sorted by (created_at ASC, task_id ASC).
    async fn list_by_parent_run(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError>;

    /// Check whether any child task of a parent run is still non-terminal.
    /// Used by recovery to detect stale waiting_dependency states.
    async fn any_non_terminal_children(&self, parent_run_id: &RunId) -> Result<bool, StoreError>;
}

/// Read model for expired task leases.
#[async_trait::async_trait]
pub trait TaskLeaseExpiredReadModel: Send + Sync {
    /// List tasks whose lease has expired as of `now_ms`.
    async fn list_expired(&self, now_ms: u64) -> Result<Vec<TaskRecord>, crate::error::StoreError>;
}
