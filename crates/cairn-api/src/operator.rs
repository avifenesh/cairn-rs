use async_trait::async_trait;
use cairn_domain::ids::{ApprovalId, RunId, TaskId};
use cairn_domain::tenancy::ProjectKey;
use cairn_runtime::error::RuntimeError;
use cairn_store::projections::{ApprovalRecord, MailboxRecord, RunRecord, TaskRecord};

use crate::endpoints::ListQuery;
use crate::http::ListResponse;

/// Operator command endpoints for mutation operations.
///
/// These endpoints accept operator commands and forward them to
/// the runtime service layer. They complement the read-only
/// `RuntimeReadEndpoints` from Week 2.
#[async_trait]
pub trait OperatorCommandEndpoints: Send + Sync {
    /// `POST /v1/approvals/:id/approve`
    async fn approve(&self, approval_id: &ApprovalId) -> Result<ApprovalRecord, RuntimeError>;

    /// `POST /v1/approvals/:id/deny`
    async fn deny(&self, approval_id: &ApprovalId) -> Result<ApprovalRecord, RuntimeError>;

    /// `POST /v1/tasks/:id/cancel`
    async fn cancel_task(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError>;
}

/// Operator read endpoints for run detail, approval inbox, and mailbox visibility.
#[async_trait]
pub trait OperatorReadEndpoints: Send + Sync {
    /// Detailed run view with linked tasks.
    async fn get_run_detail(&self, run_id: &RunId) -> Result<Option<RunDetail>, RuntimeError>;

    /// Approval inbox: pending approvals with context.
    async fn list_pending_approvals(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<ApprovalRecord>, RuntimeError>;

    /// Mailbox messages for a run.
    async fn list_mailbox_by_run(
        &self,
        run_id: &RunId,
        query: &ListQuery,
    ) -> Result<ListResponse<MailboxRecord>, RuntimeError>;

    /// Mailbox messages for a task.
    async fn list_mailbox_by_task(
        &self,
        task_id: &TaskId,
        query: &ListQuery,
    ) -> Result<ListResponse<MailboxRecord>, RuntimeError>;
}

/// Rich run detail combining run record with linked tasks.
#[derive(Clone, Debug)]
pub struct RunDetail {
    pub run: RunRecord,
    pub tasks: Vec<TaskRecord>,
}

/// Action to apply across a batch of approvals (RFC 010).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BulkApprovalAction {
    /// Approve every approval in the batch.
    ApproveAll,
    /// Deny every approval in the batch.
    DenyAll,
    /// Defer every approval in the batch for later review.
    DeferAll,
}

/// Request body for a bulk approval queue action (RFC 010).
///
/// Operators use this to drain or defer an entire approval queue in one
/// call rather than issuing individual approve/deny requests.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BulkApprovalRequest {
    /// IDs of the approvals to act on.
    pub approval_ids: Vec<String>,
    /// Action to apply uniformly to all listed approvals.
    pub action: BulkApprovalAction,
    /// Optional operator-supplied reason recorded on each approval.
    pub reason: Option<String>,
}

/// Response for a bulk approval queue action (RFC 010).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BulkApprovalResponse {
    /// Number of approvals successfully acted on.
    pub processed: usize,
    /// Number of approvals skipped (e.g. already resolved or not found).
    pub skipped: usize,
    /// Per-item error messages for any approvals that failed to process.
    pub errors: Vec<String>,
}

/// Operator-layer error for approval actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperatorError {
    /// The referenced approval, run, or task was not found.
    NotFound(String),
    /// The request was rejected by a policy or validation rule.
    PolicyDenied(String),
    /// An unexpected internal failure occurred.
    Internal(String),
}

impl std::fmt::Display for OperatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OperatorError::NotFound(id) => write!(f, "not found: {id}"),
            OperatorError::PolicyDenied(reason) => write!(f, "policy denied: {reason}"),
            OperatorError::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

/// Bulk and deferred approval actions for operator workflows (RFC 010).
///
/// Extends `OperatorCommandEndpoints` with high-throughput queue management:
/// operators can drain or defer entire approval backlogs in a single call.
#[async_trait]
pub trait OperatorApprovalActions: Send + Sync {
    /// Approve multiple approvals in one call.
    ///
    /// Default stub — returns a zero-count success response.
    async fn bulk_approve(
        &self,
        _approval_ids: Vec<String>,
        _reason: Option<String>,
    ) -> Result<BulkApprovalResponse, OperatorError> {
        Ok(BulkApprovalResponse {
            processed: 0,
            skipped: 0,
            errors: vec![],
        })
    }

    /// Deny multiple approvals in one call.
    ///
    /// Default stub — returns a zero-count success response.
    async fn bulk_deny(
        &self,
        _approval_ids: Vec<String>,
        _reason: Option<String>,
    ) -> Result<BulkApprovalResponse, OperatorError> {
        Ok(BulkApprovalResponse {
            processed: 0,
            skipped: 0,
            errors: vec![],
        })
    }

    /// Defer a single approval until a specified wall-clock time (ms since epoch).
    ///
    /// Default stub — always succeeds without side effects.
    async fn defer_approval(
        &self,
        _approval_id: String,
        _defer_until: u64,
    ) -> Result<(), OperatorError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::{RunId, SessionId};
    use cairn_domain::lifecycle::RunState;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_store::projections::RunRecord;

    #[test]
    fn run_detail_construction() {
        let detail = RunDetail {
            run: RunRecord {
                run_id: RunId::new("run_1"),
                session_id: SessionId::new("sess_1"),
                parent_run_id: None,
                project: ProjectKey::new("t", "w", "p"),
                state: RunState::Running,
                prompt_release_id: None,
                agent_role_id: None,
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
                version: 1,
                created_at: 1000,
                updated_at: 1001,
                completion_summary: None,
                completion_verification: None,
                completion_annotated_at_ms: None,
            },
            tasks: vec![],
        };
        assert_eq!(detail.run.state, RunState::Running);
        assert!(detail.tasks.is_empty());
    }
}
