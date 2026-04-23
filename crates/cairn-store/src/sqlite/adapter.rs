use async_trait::async_trait;
use cairn_domain::tenancy::ProjectKey;
use cairn_domain::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationRecord};
use cairn_domain::{
    ApprovalDecision, ApprovalId, ApprovalRequirement, CheckpointDisposition, CheckpointId,
    FailureClass, MailboxMessageId, RunId, RunState, SessionId, SessionState, TaskId, TaskState,
    ToolInvocationId,
};
use serde::de::DeserializeOwned;
use sqlx::SqlitePool;

use crate::db::{Backend, DbAdapter};
use crate::error::StoreError;
use crate::projections::{
    ApprovalReadModel, ApprovalRecord, CheckpointReadModel, CheckpointRecord,
    CheckpointStrategyReadModel, MailboxReadModel, MailboxRecord, RunReadModel, RunRecord,
    SessionReadModel, SessionRecord, TaskReadModel, TaskRecord, ToolInvocationReadModel,
};

/// SQLite-backed database adapter for local-mode.
pub struct SqliteAdapter {
    pool: SqlitePool,
}

impl SqliteAdapter {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Create an in-memory SQLite database with schema applied.
    /// Useful for tests.
    pub async fn in_memory() -> Result<Self, StoreError> {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .map_err(|e| StoreError::Connection(e.to_string()))?;

        let adapter = Self::new(pool);
        adapter.migrate().await?;
        Ok(adapter)
    }
}

#[async_trait]
impl DbAdapter for SqliteAdapter {
    fn backend(&self) -> Backend {
        Backend::Sqlite
    }

    async fn health_check(&self) -> Result<(), StoreError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Connection(e.to_string()))?;
        Ok(())
    }

    async fn migrate(&self) -> Result<(), StoreError> {
        // Enable WAL mode for better concurrency.
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Migration(e.to_string()))?;

        // Enable foreign keys.
        sqlx::query("PRAGMA foreign_keys=ON")
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Migration(e.to_string()))?;

        // Apply the full schema in one shot. This avoids brittle semicolon splitting
        // for FTS and other multi-line DDL.
        sqlx::raw_sql(super::schema::SCHEMA_SQL)
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Migration(format!("schema: {e}")))?;

        // Column adds for pre-existing databases. SQLite lacks `ADD COLUMN IF
        // NOT EXISTS`, so we consult `pragma_table_info` to decide whether the
        // column is already present before running ALTER — this avoids relying
        // on brittle substring matches against sqlx error strings that differ
        // across SQLite and sqlx versions / locales.
        let archived_at_exists = sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM pragma_table_info('workspaces') \
               WHERE name = 'archived_at' LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Migration(format!("pragma workspaces.archived_at: {e}")))?
        .is_some();

        if !archived_at_exists {
            let stmt = "ALTER TABLE workspaces ADD COLUMN archived_at INTEGER";
            sqlx::query(stmt)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Migration(format!("{stmt}: {e}")))?;
        }

        Ok(())
    }
}

#[async_trait]
impl SessionReadModel for SqliteAdapter {
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
             ORDER BY updated_at DESC
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
impl RunReadModel for SqliteAdapter {
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
        let row: (i64,) = sqlx::query_as(
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

        Ok(row.0 != 0)
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
        let state_str = serde_json::to_value(state)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{state:?}").to_lowercase());
        let rows = sqlx::query_as::<_, RunRow>(
            "SELECT run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                    state, failure_class, version, created_at, updated_at
             FROM runs
             WHERE state = $1
             ORDER BY created_at ASC
             LIMIT $2",
        )
        .bind(&state_str)
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
        let terminal_states = ["completed", "failed", "canceled", "dead_lettered"];
        let placeholders = terminal_states
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", i + 4))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                    state, failure_class, version, created_at, updated_at
             FROM runs
             WHERE tenant_id = $1 AND workspace_id = $2 AND project_id = $3
               AND state NOT IN ({placeholders})
             ORDER BY created_at ASC
             LIMIT ${}",
            4 + terminal_states.len()
        );
        let mut q = sqlx::query_as::<_, RunRow>(&sql)
            .bind(project.tenant_id.as_str())
            .bind(project.workspace_id.as_str())
            .bind(project.project_id.as_str());
        for s in &terminal_states {
            q = q.bind(*s);
        }
        q = q.bind(limit as i64);
        q.fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Internal(e.to_string()))?
            .into_iter()
            .map(RunRow::into_record)
            .collect()
    }

    async fn list_by_parent_run(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        // Served by `idx_runs_parent` (V003__create_runs.sql partial
        // index on `parent_run_id WHERE NOT NULL`).
        let rows = sqlx::query_as::<_, RunRow>(
            "SELECT run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                    state, failure_class, version, created_at, updated_at
             FROM runs
             WHERE parent_run_id = $1
             ORDER BY created_at ASC, run_id ASC
             LIMIT $2",
        )
        .bind(parent_run_id.as_str())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;
        rows.into_iter().map(RunRow::into_record).collect()
    }
}

#[async_trait]
impl TaskReadModel for SqliteAdapter {
    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, StoreError> {
        let row = sqlx::query_as::<_, TaskRow>(
            "SELECT task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id, session_id,
                    state, failure_class, lease_owner, lease_expires_at, title, description, version, created_at, updated_at
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
            "SELECT task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id, session_id,
                    state, failure_class, lease_owner, lease_expires_at, title, description, version, created_at, updated_at
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
            "SELECT task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id, session_id,
                    state, failure_class, lease_owner, lease_expires_at, title, description, version, created_at, updated_at
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
            "SELECT task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id, session_id,
                    state, failure_class, lease_owner, lease_expires_at, title, description, version, created_at, updated_at
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
        let row: (i64,) = sqlx::query_as(
            "SELECT EXISTS(
                SELECT 1 FROM tasks
                WHERE parent_run_id = $1
                  AND state NOT IN ('completed', 'failed', 'canceled', 'dead_lettered')
            ) AS has_non_terminal",
        )
        .bind(parent_run_id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        Ok(row.0 != 0)
    }
}

#[async_trait]
impl ApprovalReadModel for SqliteAdapter {
    async fn get(&self, approval_id: &ApprovalId) -> Result<Option<ApprovalRecord>, StoreError> {
        let row = sqlx::query_as::<_, ApprovalRow>(
            "SELECT approval_id, tenant_id, workspace_id, project_id, run_id, task_id,
                    requirement, decision, title, description, version, created_at, updated_at
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
                    requirement, decision, title, description, version, created_at, updated_at
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

    async fn list_all(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, StoreError> {
        let rows = sqlx::query_as::<_, ApprovalRow>(
            "SELECT approval_id, tenant_id, workspace_id, project_id, run_id, task_id,
                    requirement, decision, title, description, version, created_at, updated_at
             FROM approvals
             WHERE tenant_id = $1 AND workspace_id = $2 AND project_id = $3
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
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM approvals WHERE run_id = $1 AND decision IS NULL")
                .bind(run_id.as_str())
                .fetch_one(&self.pool)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
        Ok(count.0 > 0)
    }
}

#[async_trait]
impl CheckpointStrategyReadModel for SqliteAdapter {
    async fn get_by_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<cairn_domain::CheckpointStrategy>, StoreError> {
        // Checkpoint strategies are stored as events; query the strategies table if it exists,
        // otherwise return None (strategy not configured).
        let _ = run_id;
        Ok(None)
    }
}

#[async_trait]
impl CheckpointReadModel for SqliteAdapter {
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
impl MailboxReadModel for SqliteAdapter {
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

    async fn list_pending(
        &self,
        _now_ms: u64,
        _limit: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        // SQLite migration for deliver_at_ms column is out of scope; stub returns empty.
        Ok(vec![])
    }
}

#[async_trait]
impl ToolInvocationReadModel for SqliteAdapter {
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
    target: String,
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
        let project = project_key_from_parts(self.tenant_id, self.workspace_id, self.project_id);
        Ok(ToolInvocationRecord {
            invocation_id: ToolInvocationId::new(self.invocation_id),
            project,
            session_id: self.session_id.map(SessionId::new),
            run_id: self.run_id.map(RunId::new),
            task_id: self.task_id.map(TaskId::new),
            target: serde_json::from_str(&self.target)
                .map_err(|e| StoreError::Serialization(e.to_string()))?,
            execution_class: parse_string_enum(&self.execution_class)?,
            prompt_release_id: None,
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
        let project = project_key_from_parts(self.tenant_id, self.workspace_id, self.project_id);
        Ok(SessionRecord {
            session_id: SessionId::new(self.session_id),
            project,
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
        let project = project_key_from_parts(self.tenant_id, self.workspace_id, self.project_id);
        Ok(RunRecord {
            run_id: RunId::new(self.run_id),
            session_id: SessionId::new(self.session_id),
            parent_run_id: self.parent_run_id.map(RunId::new),
            project,
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
    session_id: Option<String>,
    state: String,
    failure_class: Option<String>,
    lease_owner: Option<String>,
    lease_expires_at: Option<i64>,
    title: Option<String>,
    description: Option<String>,
    version: i64,
    created_at: i64,
    updated_at: i64,
}

impl TaskRow {
    fn into_record(self) -> Result<TaskRecord, StoreError> {
        let project = project_key_from_parts(self.tenant_id, self.workspace_id, self.project_id);
        Ok(TaskRecord {
            task_id: TaskId::new(self.task_id),
            project,
            parent_run_id: self.parent_run_id.map(RunId::new),
            parent_task_id: self.parent_task_id.map(TaskId::new),
            session_id: self.session_id.map(cairn_domain::SessionId::new),
            state: parse_string_enum::<TaskState>(&self.state)?,
            prompt_release_id: None,
            failure_class: self
                .failure_class
                .as_deref()
                .map(parse_string_enum::<FailureClass>)
                .transpose()?,
            lease_owner: self.lease_owner,
            lease_expires_at: self.lease_expires_at.map(|value| value as u64),
            title: self.title,
            description: self.description,
            pause_reason: None,
            resume_trigger: None,
            retry_count: 0,
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
    title: Option<String>,
    description: Option<String>,
    version: i64,
    created_at: i64,
    updated_at: i64,
}

impl ApprovalRow {
    fn into_record(self) -> Result<ApprovalRecord, StoreError> {
        let project = project_key_from_parts(self.tenant_id, self.workspace_id, self.project_id);
        Ok(ApprovalRecord {
            approval_id: ApprovalId::new(self.approval_id),
            project,
            run_id: self.run_id.map(RunId::new),
            task_id: self.task_id.map(TaskId::new),
            requirement: parse_string_enum::<ApprovalRequirement>(&self.requirement)?,
            decision: self
                .decision
                .as_deref()
                .map(parse_string_enum::<ApprovalDecision>)
                .transpose()?,
            title: self.title,
            description: self.description,
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
        let project = project_key_from_parts(self.tenant_id, self.workspace_id, self.project_id);
        Ok(CheckpointRecord {
            checkpoint_id: CheckpointId::new(self.checkpoint_id),
            project,
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
        let project = project_key_from_parts(self.tenant_id, self.workspace_id, self.project_id);
        Ok(MailboxRecord {
            message_id: MailboxMessageId::new(self.message_id),
            project,
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

fn project_key_from_parts(
    tenant_id: String,
    workspace_id: String,
    project_id: String,
) -> ProjectKey {
    ProjectKey::new(tenant_id, workspace_id, project_id)
}

#[async_trait]
impl crate::projections::FfLeaseHistoryCursorStore for SqliteAdapter {
    async fn get(
        &self,
        partition_id: &str,
        execution_id: &str,
    ) -> Result<Option<crate::projections::FfLeaseHistoryCursor>, StoreError> {
        let row: Option<(String, String, String, i64)> = sqlx::query_as(
            "SELECT partition_id, execution_id, last_stream_id, updated_at_ms
             FROM ff_lease_history_cursors
             WHERE partition_id = ?1 AND execution_id = ?2",
        )
        .bind(partition_id)
        .bind(execution_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;
        Ok(
            row.map(|(p, e, s, ts)| crate::projections::FfLeaseHistoryCursor {
                partition_id: p,
                execution_id: e,
                last_stream_id: s,
                updated_at_ms: ts as u64,
            }),
        )
    }

    async fn list_by_partition(
        &self,
        partition_id: &str,
    ) -> Result<Vec<crate::projections::FfLeaseHistoryCursor>, StoreError> {
        let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
            "SELECT partition_id, execution_id, last_stream_id, updated_at_ms
             FROM ff_lease_history_cursors
             WHERE partition_id = ?1",
        )
        .bind(partition_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|(p, e, s, ts)| crate::projections::FfLeaseHistoryCursor {
                partition_id: p,
                execution_id: e,
                last_stream_id: s,
                updated_at_ms: ts as u64,
            })
            .collect())
    }

    async fn upsert(
        &self,
        cursor: &crate::projections::FfLeaseHistoryCursor,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO ff_lease_history_cursors
                (partition_id, execution_id, last_stream_id, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (partition_id, execution_id) DO UPDATE
                SET last_stream_id = excluded.last_stream_id,
                    updated_at_ms = excluded.updated_at_ms",
        )
        .bind(&cursor.partition_id)
        .bind(&cursor.execution_id)
        .bind(&cursor.last_stream_id)
        .bind(cursor.updated_at_ms as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn delete(&self, partition_id: &str, execution_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            "DELETE FROM ff_lease_history_cursors
             WHERE partition_id = ?1 AND execution_id = ?2",
        )
        .bind(partition_id)
        .bind(execution_id)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::tool_invocation::{ToolInvocationState, ToolInvocationTarget};
    use cairn_domain::{ApprovalDecision, CheckpointDisposition, TaskState};

    #[tokio::test]
    async fn sqlite_adapter_reads_tool_invocations_in_request_order() {
        let adapter = SqliteAdapter::in_memory().await.unwrap();
        let target = serde_json::to_string(&ToolInvocationTarget::Builtin {
            tool_name: "fs.read".to_owned(),
        })
        .unwrap();

        sqlx::query(
            "INSERT INTO sessions (
                session_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("sess_1")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind("open")
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO runs (
                run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                state, failure_class, version, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("run_1")
        .bind("sess_1")
        .bind(Option::<&str>::None)
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind("running")
        .bind(Option::<&str>::None)
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO tool_invocations (
                invocation_id, tenant_id, workspace_id, project_id, session_id, run_id, task_id,
                target, execution_class, state, outcome, error_message, version,
                requested_at_ms, started_at_ms, finished_at_ms, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("tool_new")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind(Some("sess_1"))
        .bind(Some("run_1"))
        .bind(Option::<&str>::None)
        .bind(&target)
        .bind("sandboxed_process")
        .bind("canceled")
        .bind(Some("canceled"))
        .bind(Some("canceled"))
        .bind(2_i64)
        .bind(200_i64)
        .bind(Some(201_i64))
        .bind(Some(205_i64))
        .bind(200_i64)
        .bind(205_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO tool_invocations (
                invocation_id, tenant_id, workspace_id, project_id, session_id, run_id, task_id,
                target, execution_class, state, outcome, error_message, version,
                requested_at_ms, started_at_ms, finished_at_ms, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("tool_old")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind(Some("sess_1"))
        .bind(Some("run_1"))
        .bind(Option::<&str>::None)
        .bind(&target)
        .bind("supervised_process")
        .bind("started")
        .bind(Option::<&str>::None)
        .bind(Option::<&str>::None)
        .bind(1_i64)
        .bind(100_i64)
        .bind(Some(101_i64))
        .bind(Option::<i64>::None)
        .bind(100_i64)
        .bind(101_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        let records = ToolInvocationReadModel::list_by_run(&adapter, &RunId::new("run_1"), 10, 0)
            .await
            .unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].invocation_id.as_str(), "tool_old");
        assert_eq!(records[1].invocation_id.as_str(), "tool_new");
        assert_eq!(records[1].state, ToolInvocationState::Canceled);
        assert_eq!(
            records[1].outcome,
            Some(ToolInvocationOutcomeKind::Canceled)
        );
        assert_eq!(records[1].error_message.as_deref(), Some("canceled"));
    }

    #[tokio::test]
    async fn sqlite_adapter_reads_sessions_and_runs() {
        let adapter = SqliteAdapter::in_memory().await.unwrap();

        sqlx::query(
            "INSERT INTO sessions (
                session_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?), (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("sess_1")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind("open")
        .bind(1_i64)
        .bind(10_i64)
        .bind(10_i64)
        .bind("sess_2")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind("completed")
        .bind(2_i64)
        .bind(20_i64)
        .bind(25_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO runs (
                run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                state, failure_class, version, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?), (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("run_root")
        .bind("sess_1")
        .bind(Option::<&str>::None)
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind("running")
        .bind(Option::<&str>::None)
        .bind(1_i64)
        .bind(100_i64)
        .bind(101_i64)
        .bind("run_child")
        .bind("sess_1")
        .bind(Some("run_root"))
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind("completed")
        .bind(Option::<&str>::None)
        .bind(2_i64)
        .bind(110_i64)
        .bind(120_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        let sessions = SessionReadModel::list_by_project(
            &adapter,
            &ProjectKey::new("tenant", "workspace", "project"),
            10,
            0,
        )
        .await
        .unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id.as_str(), "sess_1");

        let runs = RunReadModel::list_by_session(&adapter, &SessionId::new("sess_1"), 10, 0)
            .await
            .unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id.as_str(), "run_root");
        assert!(
            RunReadModel::any_non_terminal(&adapter, &SessionId::new("sess_1"))
                .await
                .unwrap()
        );

        let latest_root = RunReadModel::latest_root_run(&adapter, &SessionId::new("sess_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest_root.run_id.as_str(), "run_root");
    }

    #[tokio::test]
    async fn sqlite_adapter_reads_task_approval_checkpoint_and_mailbox_models() {
        let adapter = SqliteAdapter::in_memory().await.unwrap();
        let project = ProjectKey::new("tenant", "workspace", "project");

        sqlx::query(
            "INSERT INTO sessions (
                session_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("sess_1")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind("open")
        .bind(1_i64)
        .bind(10_i64)
        .bind(10_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO runs (
                run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id,
                state, failure_class, version, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("run_root")
        .bind("sess_1")
        .bind(Option::<&str>::None)
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind("running")
        .bind(Option::<&str>::None)
        .bind(1_i64)
        .bind(20_i64)
        .bind(20_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO tasks (
                task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id,
                state, failure_class, lease_owner, lease_expires_at, lease_version, version, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?), (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("task_expired")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind(Some("run_root"))
        .bind(Option::<&str>::None)
        .bind("leased")
        .bind(Option::<&str>::None)
        .bind(Some("worker-a"))
        .bind(Some(50_i64))
        .bind(1_i64)
        .bind(1_i64)
        .bind(30_i64)
        .bind(40_i64)
        .bind("task_queued")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind(Some("run_root"))
        .bind(Option::<&str>::None)
        .bind("queued")
        .bind(Option::<&str>::None)
        .bind(Option::<&str>::None)
        .bind(Option::<i64>::None)
        .bind(0_i64)
        .bind(1_i64)
        .bind(35_i64)
        .bind(35_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO approvals (
                approval_id, tenant_id, workspace_id, project_id, run_id, task_id,
                requirement, decision, title, description, version, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?), (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("approval_pending")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind(Some("run_root"))
        .bind(Some("task_queued"))
        .bind("required")
        .bind(Option::<&str>::None)
        .bind(Option::<&str>::None) // title
        .bind(Option::<&str>::None) // description
        .bind(1_i64)
        .bind(40_i64)
        .bind(40_i64)
        .bind("approval_resolved")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind(Some("run_root"))
        .bind(Some("task_expired"))
        .bind("required")
        .bind(Some("approved"))
        .bind(Option::<&str>::None) // title
        .bind(Option::<&str>::None) // description
        .bind(2_i64)
        .bind(45_i64)
        .bind(46_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO checkpoints (
                checkpoint_id, tenant_id, workspace_id, project_id, run_id, disposition, version, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?), (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("cp_old")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind("run_root")
        .bind("superseded")
        .bind(1_i64)
        .bind(50_i64)
        .bind("cp_latest")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind("run_root")
        .bind("latest")
        .bind(2_i64)
        .bind(60_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO mailbox_messages (
                message_id, tenant_id, workspace_id, project_id, run_id, task_id, version, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?), (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("msg_run")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind(Some("run_root"))
        .bind(Option::<&str>::None)
        .bind(1_i64)
        .bind(70_i64)
        .bind("msg_task")
        .bind("tenant")
        .bind("workspace")
        .bind("project")
        .bind(Option::<&str>::None)
        .bind(Some("task_expired"))
        .bind(1_i64)
        .bind(80_i64)
        .execute(adapter.pool())
        .await
        .unwrap();

        let queued = TaskReadModel::list_by_state(&adapter, &project, TaskState::Queued, 10)
            .await
            .unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].task_id.as_str(), "task_queued");

        let expired = TaskReadModel::list_expired_leases(&adapter, 60, 10)
            .await
            .unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].task_id.as_str(), "task_expired");

        let pending = ApprovalReadModel::list_pending(&adapter, &project, 10, 0)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].approval_id.as_str(), "approval_pending");
        assert_eq!(pending[0].decision, None);

        let resolved = ApprovalReadModel::get(&adapter, &ApprovalId::new("approval_resolved"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.decision, Some(ApprovalDecision::Approved));

        let latest = CheckpointReadModel::latest_for_run(&adapter, &RunId::new("run_root"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.checkpoint_id.as_str(), "cp_latest");
        assert_eq!(latest.disposition, CheckpointDisposition::Latest);

        let checkpoints = CheckpointReadModel::list_by_run(&adapter, &RunId::new("run_root"), 10)
            .await
            .unwrap();
        assert_eq!(checkpoints.len(), 2);
        assert_eq!(checkpoints[0].checkpoint_id.as_str(), "cp_latest");

        let run_messages = MailboxReadModel::list_by_run(&adapter, &RunId::new("run_root"), 10, 0)
            .await
            .unwrap();
        assert_eq!(run_messages.len(), 1);
        assert_eq!(run_messages[0].message_id.as_str(), "msg_run");

        let task_messages =
            MailboxReadModel::list_by_task(&adapter, &TaskId::new("task_expired"), 10, 0)
                .await
                .unwrap();
        assert_eq!(task_messages.len(), 1);
        assert_eq!(task_messages[0].message_id.as_str(), "msg_task");
    }
}
