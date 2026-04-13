//! Approval service boundary per RFC 005.
//!
//! Approvals gate runtime execution. Runs/tasks enter waiting_approval
//! when blocked on a human decision.

use async_trait::async_trait;
use cairn_domain::{ApprovalDecision, ApprovalId, ApprovalRequirement, ProjectKey, RunId, TaskId};
use cairn_store::projections::ApprovalRecord;

use crate::error::RuntimeError;

/// Approval service boundary.
#[async_trait]
pub trait ApprovalService: Send + Sync {
    /// Request approval (blocks the associated run/task).
    async fn request(
        &self,
        project: &ProjectKey,
        approval_id: ApprovalId,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
        requirement: ApprovalRequirement,
    ) -> Result<ApprovalRecord, RuntimeError>;

    /// Request approval with context (title + description for the operator).
    async fn request_with_context(
        &self,
        project: &ProjectKey,
        approval_id: ApprovalId,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
        requirement: ApprovalRequirement,
        _title: Option<String>,
        _description: Option<String>,
    ) -> Result<ApprovalRecord, RuntimeError> {
        // Default: delegate to request() without context (backwards compat).
        self.request(project, approval_id, run_id, task_id, requirement)
            .await
    }

    /// Get an approval by ID.
    async fn get(&self, approval_id: &ApprovalId) -> Result<Option<ApprovalRecord>, RuntimeError>;

    /// Resolve an approval (approved or rejected).
    async fn resolve(
        &self,
        approval_id: &ApprovalId,
        decision: ApprovalDecision,
    ) -> Result<ApprovalRecord, RuntimeError>;

    /// List pending approvals for a project (operator inbox).
    async fn list_pending(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, RuntimeError>;

    /// List all approvals (pending + resolved) for a project.
    async fn list_all(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, RuntimeError>;
}

#[cfg(test)]
mod tests {
    use cairn_domain::ApprovalDecision;

    #[test]
    fn approval_decisions_are_distinct() {
        assert_ne!(ApprovalDecision::Approved, ApprovalDecision::Rejected);
    }
}
