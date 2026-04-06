use async_trait::async_trait;
use cairn_domain::tenancy::ProjectKey;
use cairn_domain::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationRecord};
use cairn_domain::{
    ApprovalDecision, ApprovalId, ApprovalRequirement, CheckpointDisposition, CheckpointId,
    FailureClass, MailboxMessageId, RunId, RunState, SessionId, SessionState, TaskId, TaskState,
    ToolInvocationId,
};
use serde::de::DeserializeOwned;
use sqlx::PgPool;

use crate::db::{Backend, DbAdapter};
use crate::error::StoreError;
use crate::projections::{
    ApprovalReadModel, ApprovalRecord, CheckpointReadModel, CheckpointRecord,
    CheckpointStrategyReadModel, MailboxReadModel, MailboxRecord, RunReadModel, RunRecord,
    SessionReadModel, SessionRecord, TaskReadModel, TaskRecord, ToolInvocationReadModel,
};

/// Postgres-backed database adapter.
///
/// Wraps a `sqlx::PgPool` and provides the transactional boundary
/// that ties event-log appends to synchronous projection updates.
pub struct PgAdapter {
    pool: PgPool,
}

impl PgAdapter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl DbAdapter for PgAdapter {
    fn backend(&self) -> Backend {
        Backend::Postgres
    }

    async fn health_check(&self) -> Result<(), StoreError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Connection(e.to_string()))?;
        Ok(())
    }

    async fn migrate(&self) -> Result<(), StoreError> {
        use super::migration_runner::PgMigrationRunner;

        let runner = PgMigrationRunner::new(self.pool.clone());
        runner.run_pending().await?;
        Ok(())
    }
}

#[async_trait]
impl SessionReadModel for PgAdapter {
    async fn get(&self, session_id: &SessionId) -> Result<Option<SessionRecord>, StoreError> {
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT session_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at
             FROM sessions
             WHERE session_id = $1",
        )
        .bind(session_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        row.map(SessionRow::into_record).transpose()
    }

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SessionRecord>, StoreError> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT session_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at
             FROM sessions
             WHERE tenant_id = $1 AND workspace_id = $2 AND project_id = $3
             ORDER BY created_at ASC, session_id ASC
             LIMIT $4 OFFSET $5",
        )
        .bind(project.tenant_id.as_str())
        .bind(project.workspace_id.as_str())
        .bind(project.project_id.as_str())
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(SessionRow::into_record).collect()
    }

    async fn list_active(&self, limit: usize) -> Result<Vec<SessionRecord>, StoreError> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT session_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at
             FROM sessions
             WHERE state = 'open'
             ORDER BY updated_at DESC, session_id ASC
             LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;
        rows.into_iter().map(SessionRow::into_record).collect()
    }
}

#[async_trait]
impl RunReadModel for PgAdapter {
    async fn get(&self, run_id: &RunId) -> Result<Option<RunRecord>, StoreError> {
        let row = sqlx::query_as::<_, RunRow>(
            "SELECT run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                    state, failure_class, version, created_at, updated_at
             FROM runs
             WHERE run_id = $1",
        )
        .bind(run_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        row.map(RunRow::into_record).transpose()
    }

    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query_as::<_, RunRow>(
            "SELECT run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                    state, failure_class, version, created_at, updated_at
             FROM runs
             WHERE session_id = $1
             ORDER BY created_at ASC, run_id ASC
             LIMIT $2 OFFSET $3",
        )
        .bind(session_id.as_str())
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(RunRow::into_record).collect()
    }

    async fn any_non_terminal(&self, session_id: &SessionId) -> Result<bool, StoreError> {
        let row: (bool,) = sqlx::query_as(
            "SELECT EXISTS(
                SELECT 1 FROM runs
                WHERE session_id = $1
                  AND state NOT IN ('completed', 'failed', 'canceled')
             )",
        )
        .bind(session_id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        Ok(row.0)
    }

    async fn latest_root_run(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<RunRecord>, StoreError> {
        let row = sqlx::query_as::<_, RunRow>(
            "SELECT run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                    state, failure_class, version, created_at, updated_at
             FROM runs
             WHERE session_id = $1 AND parent_run_id IS NULL
             ORDER BY created_at DESC, run_id DESC
             LIMIT 1",
        )
        .bind(session_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        row.map(RunRow::into_record).transpose()
    }

    async fn list_by_state(
        &self,
        state: RunState,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query_as::<_, RunRow>(
            "SELECT run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                    state, failure_class, version, created_at, updated_at
             FROM runs
             WHERE state = $1
             ORDER BY created_at ASC, run_id ASC
             LIMIT $2",
        )
        .bind(enum_string(&state)?)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(RunRow::into_record).collect()
    }

    async fn list_active_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query_as::<_, RunRow>(
            "SELECT run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                    state, failure_class, version, created_at, updated_at
             FROM runs
             WHERE tenant_id = $1 AND workspace_id = $2 AND project_id = $3
               AND state NOT IN ('completed', 'failed', 'canceled', 'dead_lettered')
             ORDER BY created_at ASC, run_id ASC
             LIMIT $4",
        )
        .bind(project.tenant_id.as_str())
        .bind(project.workspace_id.as_str())
        .bind(project.project_id.as_str())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;
        rows.into_iter().map(RunRow::into_record).collect()
    }
}

#[async_trait]
impl TaskReadModel for PgAdapter {
    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, StoreError> {
        let row = sqlx::query_as::<_, TaskRow>(
            "SELECT task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id,
                    state, failure_class, lease_owner, lease_expires_at, version, created_at, updated_at
             FROM tasks
             WHERE task_id = $1",
        )
        .bind(task_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        row.map(TaskRow::into_record).transpose()
    }

    async fn list_by_state(
        &self,
        project: &ProjectKey,
        task_state: TaskState,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let rows = sqlx::query_as::<_, TaskRow>(
            "SELECT task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id,
                    state, failure_class, lease_owner, lease_expires_at, version, created_at, updated_at
             FROM tasks
             WHERE tenant_id = $1 AND workspace_id = $2 AND project_id = $3 AND state = $4
             ORDER BY created_at ASC, task_id ASC
             LIMIT $5",
        )
        .bind(project.tenant_id.as_str())
        .bind(project.workspace_id.as_str())
        .bind(project.project_id.as_str())
        .bind(enum_string(&task_state)?)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(TaskRow::into_record).collect()
    }

    async fn list_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let rows = sqlx::query_as::<_, TaskRow>(
            "SELECT task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id,
                    state, failure_class, lease_owner, lease_expires_at, version, created_at, updated_at
             FROM tasks
             WHERE state = 'leased'
               AND lease_expires_at IS NOT NULL
               AND lease_expires_at < $1
             ORDER BY lease_expires_at ASC, task_id ASC
             LIMIT $2",
        )
        .bind(now as i64)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(TaskRow::into_record).collect()
    }

    async fn list_by_parent_run(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let rows = sqlx::query_as::<_, TaskRow>(
            "SELECT task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id,
                    state, failure_class, lease_owner, lease_expires_at, version, created_at, updated_at
             FROM tasks
             WHERE parent_run_id = $1
             ORDER BY created_at ASC, task_id ASC
             LIMIT $2",
        )
        .bind(parent_run_id.as_str())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(TaskRow::into_record).collect()
    }

    async fn any_non_terminal_children(&self, parent_run_id: &RunId) -> Result<bool, StoreError> {
        let row: (bool,) = sqlx::query_as(
            "SELECT EXISTS(
                SELECT 1 FROM tasks
                WHERE parent_run_id = $1
                  AND state NOT IN ('completed', 'failed', 'canceled', 'dead_lettered')
            )",
        )
        .bind(parent_run_id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        Ok(row.0)
    }
}

#[async_trait]
impl ApprovalReadModel for PgAdapter {
    async fn get(&self, approval_id: &ApprovalId) -> Result<Option<ApprovalRecord>, StoreError> {
        let row = sqlx::query_as::<_, ApprovalRow>(
            "SELECT approval_id, tenant_id, workspace_id, project_id, run_id, task_id,
                    requirement, decision, version, created_at, updated_at
             FROM approvals
             WHERE approval_id = $1",
        )
        .bind(approval_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        row.map(ApprovalRow::into_record).transpose()
    }

    async fn list_pending(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, StoreError> {
        let rows = sqlx::query_as::<_, ApprovalRow>(
            "SELECT approval_id, tenant_id, workspace_id, project_id, run_id, task_id,
                    requirement, decision, version, created_at, updated_at
             FROM approvals
             WHERE tenant_id = $1 AND workspace_id = $2 AND project_id = $3
               AND decision IS NULL
             ORDER BY created_at ASC, approval_id ASC
             LIMIT $4 OFFSET $5",
        )
        .bind(project.tenant_id.as_str())
        .bind(project.workspace_id.as_str())
        .bind(project.project_id.as_str())
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(ApprovalRow::into_record).collect()
    }

    async fn has_pending_for_run(&self, run_id: &RunId) -> Result<bool, StoreError> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM approvals WHERE run_id = $1 AND decision IS NULL",
        )
        .bind(run_id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;
        Ok(count.0 > 0)
    }
}

#[async_trait]
impl CheckpointStrategyReadModel for PgAdapter {
    async fn get_by_run(&self, run_id: &RunId) -> Result<Option<cairn_domain::CheckpointStrategy>, StoreError> {
        let _ = run_id;
        Ok(None)
    }
}

#[async_trait]
impl CheckpointReadModel for PgAdapter {
    async fn get(
        &self,
        checkpoint_id: &CheckpointId,
    ) -> Result<Option<CheckpointRecord>, StoreError> {
        let row = sqlx::query_as::<_, CheckpointRow>(
            "SELECT checkpoint_id, tenant_id, workspace_id, project_id, run_id, disposition, version, created_at
             FROM checkpoints
             WHERE checkpoint_id = $1",
        )
        .bind(checkpoint_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        row.map(CheckpointRow::into_record).transpose()
    }

    async fn latest_for_run(&self, run_id: &RunId) -> Result<Option<CheckpointRecord>, StoreError> {
        let row = sqlx::query_as::<_, CheckpointRow>(
            "SELECT checkpoint_id, tenant_id, workspace_id, project_id, run_id, disposition, version, created_at
             FROM checkpoints
             WHERE run_id = $1 AND disposition = 'latest'
             ORDER BY created_at DESC, checkpoint_id DESC
             LIMIT 1",
        )
        .bind(run_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        row.map(CheckpointRow::into_record).transpose()
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<CheckpointRecord>, StoreError> {
        let rows = sqlx::query_as::<_, CheckpointRow>(
            "SELECT checkpoint_id, tenant_id, workspace_id, project_id, run_id, disposition, version, created_at
             FROM checkpoints
             WHERE run_id = $1
             ORDER BY created_at DESC, checkpoint_id DESC
             LIMIT $2",
        )
        .bind(run_id.as_str())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(CheckpointRow::into_record).collect()
    }
}

#[async_trait]
impl MailboxReadModel for PgAdapter {
    async fn get(
        &self,
        message_id: &MailboxMessageId,
    ) -> Result<Option<MailboxRecord>, StoreError> {
        let row = sqlx::query_as::<_, MailboxRow>(
            "SELECT message_id, tenant_id, workspace_id, project_id, run_id, task_id, version, created_at
             FROM mailbox_messages
             WHERE message_id = $1",
        )
        .bind(message_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        row.map(MailboxRow::into_record).transpose()
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        let rows = sqlx::query_as::<_, MailboxRow>(
            "SELECT message_id, tenant_id, workspace_id, project_id, run_id, task_id, version, created_at
             FROM mailbox_messages
             WHERE run_id = $1
             ORDER BY created_at ASC, message_id ASC
             LIMIT $2 OFFSET $3",
        )
        .bind(run_id.as_str())
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(MailboxRow::into_record).collect()
    }

    async fn list_by_task(
        &self,
        task_id: &TaskId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        let rows = sqlx::query_as::<_, MailboxRow>(
            "SELECT message_id, tenant_id, workspace_id, project_id, run_id, task_id, version, created_at
             FROM mailbox_messages
             WHERE task_id = $1
             ORDER BY created_at ASC, message_id ASC
             LIMIT $2 OFFSET $3",
        )
        .bind(task_id.as_str())
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(MailboxRow::into_record).collect()
    }

    async fn list_pending(&self, _now_ms: u64, _limit: usize) -> Result<Vec<MailboxRecord>, StoreError> {
        // Postgres migration for deliver_at_ms column is out of scope; stub returns empty.
        Ok(vec![])
    }
}

#[async_trait]
impl ToolInvocationReadModel for PgAdapter {
    async fn get(
        &self,
        invocation_id: &ToolInvocationId,
    ) -> Result<Option<ToolInvocationRecord>, StoreError> {
        let row = sqlx::query_as::<_, ToolInvocationRow>(
            "SELECT invocation_id, tenant_id, workspace_id, project_id, session_id, run_id, task_id,
                    target, execution_class, state, outcome, error_message, version,
                    requested_at_ms, started_at_ms, finished_at_ms
             FROM tool_invocations
             WHERE invocation_id = $1",
        )
        .bind(invocation_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        row.map(ToolInvocationRow::into_record).transpose()
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ToolInvocationRecord>, StoreError> {
        let rows = sqlx::query_as::<_, ToolInvocationRow>(
            "SELECT invocation_id, tenant_id, workspace_id, project_id, session_id, run_id, task_id,
                    target, execution_class, state, outcome, error_message, version,
                    requested_at_ms, started_at_ms, finished_at_ms
             FROM tool_invocations
             WHERE run_id = $1
             ORDER BY requested_at_ms ASC, invocation_id ASC
             LIMIT $2 OFFSET $3",
        )
        .bind(run_id.as_str())
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter()
            .map(ToolInvocationRow::into_record)
            .collect()
    }
}

#[derive(sqlx::FromRow)]
struct ToolInvocationRow {
    invocation_id: String,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    task_id: Option<String>,
    target: serde_json::Value,
    execution_class: String,
    state: String,
    outcome: Option<String>,
    error_message: Option<String>,
    version: i64,
    requested_at_ms: i64,
    started_at_ms: Option<i64>,
    finished_at_ms: Option<i64>,
}

impl ToolInvocationRow {
    fn into_record(self) -> Result<ToolInvocationRecord, StoreError> {
        Ok(ToolInvocationRecord {
            invocation_id: ToolInvocationId::new(self.invocation_id),
            project: ProjectKey::new(self.tenant_id, self.workspace_id, self.project_id),
            session_id: self.session_id.map(SessionId::new),
            run_id: self.run_id.map(RunId::new),
            task_id: self.task_id.map(TaskId::new),
            prompt_release_id: None,
            target: serde_json::from_value(self.target)
                .map_err(|e| StoreError::Serialization(e.to_string()))?,
            execution_class: parse_string_enum(&self.execution_class)?,
            state: parse_string_enum(&self.state)?,
            version: self.version as u64,
            requested_at_ms: self.requested_at_ms as u64,
            started_at_ms: self.started_at_ms.map(|value| value as u64),
            finished_at_ms: self.finished_at_ms.map(|value| value as u64),
            outcome: self
                .outcome
                .as_deref()
                .map(parse_string_enum::<ToolInvocationOutcomeKind>)
                .transpose()?,
            error_message: self.error_message,
        })
    }
}

fn parse_string_enum<T: DeserializeOwned>(raw: &str) -> Result<T, StoreError> {
    serde_json::from_value(serde_json::Value::String(raw.to_owned()))
        .map_err(|e| StoreError::Serialization(e.to_string()))
}

fn enum_string<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_value(value)
        .map_err(|e| StoreError::Serialization(e.to_string()))?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| StoreError::Serialization("enum did not serialize to string".to_owned()))
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    session_id: String,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    state: String,
    version: i64,
    created_at: i64,
    updated_at: i64,
}

impl SessionRow {
    fn into_record(self) -> Result<SessionRecord, StoreError> {
        Ok(SessionRecord {
            session_id: SessionId::new(self.session_id),
            project: ProjectKey::new(self.tenant_id, self.workspace_id, self.project_id),
            state: parse_string_enum::<SessionState>(&self.state)?,
            version: self.version as u64,
            created_at: self.created_at as u64,
            updated_at: self.updated_at as u64,
        })
    }
}

#[derive(sqlx::FromRow)]
struct RunRow {
    run_id: String,
    session_id: String,
    parent_run_id: Option<String>,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    state: String,
    failure_class: Option<String>,
    version: i64,
    created_at: i64,
    updated_at: i64,
}

impl RunRow {
    fn into_record(self) -> Result<RunRecord, StoreError> {
        Ok(RunRecord {
            run_id: RunId::new(self.run_id),
            session_id: SessionId::new(self.session_id),
            parent_run_id: self.parent_run_id.map(RunId::new),
            project: ProjectKey::new(self.tenant_id, self.workspace_id, self.project_id),
            state: parse_string_enum::<RunState>(&self.state)?,
            prompt_release_id: None,
            agent_role_id: None,
            failure_class: self
                .failure_class
                .as_deref()
                .map(parse_string_enum::<FailureClass>)
                .transpose()?,
            pause_reason: None,
            resume_trigger: None,
            version: self.version as u64,
            created_at: self.created_at as u64,
            updated_at: self.updated_at as u64,
        })
    }
}

#[derive(sqlx::FromRow)]
struct TaskRow {
    task_id: String,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    parent_run_id: Option<String>,
    parent_task_id: Option<String>,
    state: String,
    failure_class: Option<String>,
    lease_owner: Option<String>,
    lease_expires_at: Option<i64>,
    version: i64,
    created_at: i64,
    updated_at: i64,
}

impl TaskRow {
    fn into_record(self) -> Result<TaskRecord, StoreError> {
        Ok(TaskRecord {
            task_id: TaskId::new(self.task_id),
            project: ProjectKey::new(self.tenant_id, self.workspace_id, self.project_id),
            parent_run_id: self.parent_run_id.map(RunId::new),
            parent_task_id: self.parent_task_id.map(TaskId::new),
            state: parse_string_enum::<TaskState>(&self.state)?,
            prompt_release_id: None,
            failure_class: self
                .failure_class
                .as_deref()
                .map(parse_string_enum::<FailureClass>)
                .transpose()?,
            pause_reason: None,
            resume_trigger: None,
            retry_count: 0,
            lease_owner: self.lease_owner,
            lease_expires_at: self.lease_expires_at.map(|value| value as u64),
            title: None,
            description: None,
            version: self.version as u64,
            created_at: self.created_at as u64,
            updated_at: self.updated_at as u64,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ApprovalRow {
    approval_id: String,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    run_id: Option<String>,
    task_id: Option<String>,
    requirement: String,
    decision: Option<String>,
    version: i64,
    created_at: i64,
    updated_at: i64,
}

impl ApprovalRow {
    fn into_record(self) -> Result<ApprovalRecord, StoreError> {
        Ok(ApprovalRecord {
            approval_id: ApprovalId::new(self.approval_id),
            project: ProjectKey::new(self.tenant_id, self.workspace_id, self.project_id),
            run_id: self.run_id.map(RunId::new),
            task_id: self.task_id.map(TaskId::new),
            requirement: parse_string_enum::<ApprovalRequirement>(&self.requirement)?,
            title: None,
            description: None,
            decision: self
                .decision
                .as_deref()
                .map(parse_string_enum::<ApprovalDecision>)
                .transpose()?,
            version: self.version as u64,
            created_at: self.created_at as u64,
            updated_at: self.updated_at as u64,
        })
    }
}

#[derive(sqlx::FromRow)]
struct CheckpointRow {
    checkpoint_id: String,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    run_id: String,
    disposition: String,
    version: i64,
    created_at: i64,
}

impl CheckpointRow {
    fn into_record(self) -> Result<CheckpointRecord, StoreError> {
        Ok(CheckpointRecord {
            checkpoint_id: CheckpointId::new(self.checkpoint_id),
            project: ProjectKey::new(self.tenant_id, self.workspace_id, self.project_id),
            run_id: RunId::new(self.run_id),
            disposition: parse_string_enum::<CheckpointDisposition>(&self.disposition)?,
            data: None,
            version: self.version as u64,
            created_at: self.created_at as u64,
        })
    }
}

#[derive(sqlx::FromRow)]
struct MailboxRow {
    message_id: String,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    run_id: Option<String>,
    task_id: Option<String>,
    version: i64,
    created_at: i64,
}

impl MailboxRow {
    fn into_record(self) -> Result<MailboxRecord, StoreError> {
        Ok(MailboxRecord {
            message_id: MailboxMessageId::new(self.message_id),
            project: ProjectKey::new(self.tenant_id, self.workspace_id, self.project_id),
            run_id: self.run_id.map(RunId::new),
            task_id: self.task_id.map(TaskId::new),
            from_task_id: None,
            content: String::new(),
            from_run_id: None,
            deliver_at_ms: 0,
            sender: None,
            recipient: None,
            body: None,
            sent_at: None,
            delivery_status: None,
            version: self.version as u64,
            created_at: self.created_at as u64,
        })
    }
}
