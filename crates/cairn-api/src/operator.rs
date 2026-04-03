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
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
                version: 1,
                created_at: 1000,
                updated_at: 1001,
            },
            tasks: vec![],
        };
        assert_eq!(detail.run.state, RunState::Running);
        assert!(detail.tasks.is_empty());
    }
}
