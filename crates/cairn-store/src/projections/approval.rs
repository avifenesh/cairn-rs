use async_trait::async_trait;
use cairn_domain::{ApprovalDecision, ApprovalId, ApprovalRequirement, ProjectKey, RunId, TaskId};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Current-state record for an approval request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub approval_id: ApprovalId,
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub requirement: ApprovalRequirement,
    pub decision: Option<ApprovalDecision>,
    /// Product-level title for operator/SSE surfaces.
    pub title: Option<String>,
    /// Product-level description/context for operator/SSE surfaces.
    pub description: Option<String>,
    pub version: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Read-model for approval current state.
#[async_trait]
pub trait ApprovalReadModel: Send + Sync {
    async fn get(&self, approval_id: &ApprovalId) -> Result<Option<ApprovalRecord>, StoreError>;

    /// List pending approvals for a project (operator inbox).
    async fn list_pending(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, StoreError>;

    /// Check if a run has any pending (unresolved) approvals.
    async fn has_pending_for_run(&self, run_id: &RunId) -> Result<bool, StoreError>;
}
