use async_trait::async_trait;
use cairn_domain::ids::{RunId, SessionId, TaskId};
use cairn_domain::tenancy::ProjectKey;
use cairn_store::error::StoreError;
use cairn_store::projections::{
    ApprovalRecord, RunRecord, SessionRecord, TaskRecord, ToolInvocationRecord,
};
use serde::{Deserialize, Serialize};

use crate::http::ListResponse;

/// Query parameters for list endpoints.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ListQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub status: Option<String>,
    pub category: Option<String>,
}

impl ListQuery {
    pub fn effective_limit(&self) -> usize {
        self.limit.unwrap_or(50).min(200)
    }

    pub fn effective_offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

/// Handler boundary for runtime read endpoints.
///
/// Implementors wire these to specific HTTP framework handlers (axum, actix, etc.)
/// and resolve store dependencies through dependency injection.
#[async_trait]
pub trait RuntimeReadEndpoints: Send + Sync {
    /// `GET /v1/tasks` — list tasks with optional status filter.
    async fn list_tasks(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<TaskRecord>, StoreError>;

    /// `GET /v1/tasks/:id/cancel` — handled by runtime, not read endpoint.
    /// Read endpoint just provides task lookup for the cancel handler.
    async fn get_task(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, StoreError>;

    /// `GET /v1/approvals` — list approvals with optional status filter.
    async fn list_approvals(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<ApprovalRecord>, StoreError>;

    /// `GET /v1/assistant/sessions` — list sessions.
    async fn list_sessions(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<SessionRecord>, StoreError>;

    /// `GET /v1/assistant/sessions/:sessionId` — get session with messages.
    async fn get_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionRecord>, StoreError>;

    /// List runs for a session (used internally by multiple endpoints).
    async fn list_runs_by_session(
        &self,
        session_id: &SessionId,
        query: &ListQuery,
    ) -> Result<ListResponse<RunRecord>, StoreError>;

    /// List tool invocations for a run (timeline view).
    async fn list_tool_invocations_by_run(
        &self,
        run_id: &RunId,
        query: &ListQuery,
    ) -> Result<ListResponse<ToolInvocationRecord>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_query_defaults() {
        let query = ListQuery::default();
        assert_eq!(query.effective_limit(), 50);
        assert_eq!(query.effective_offset(), 0);
    }

    #[test]
    fn list_query_clamps_limit() {
        let query = ListQuery {
            limit: Some(500),
            ..Default::default()
        };
        assert_eq!(query.effective_limit(), 200);
    }
}
