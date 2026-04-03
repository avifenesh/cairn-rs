use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::StoreError;
use crate::migrations::AppliedMigration;

/// Postgres-backed migration runner.
///
/// Reads SQL files embedded at compile time from the migrations directory
/// and applies them in version order within a transaction.
pub struct PgMigrationRunner {
    pool: PgPool,
}

/// Embedded migrations from the migrations directory.
/// Each entry is (version, name, sql).
const MIGRATIONS: &[(u32, &str, &str)] = &[
    (
        1,
        "create_event_log",
        include_str!("../../migrations/V001__create_event_log.sql"),
    ),
    (
        2,
        "create_sessions",
        include_str!("../../migrations/V002__create_sessions.sql"),
    ),
    (
        3,
        "create_runs",
        include_str!("../../migrations/V003__create_runs.sql"),
    ),
    (
        4,
        "create_tasks",
        include_str!("../../migrations/V004__create_tasks.sql"),
    ),
    (
        5,
        "create_approvals",
        include_str!("../../migrations/V005__create_approvals.sql"),
    ),
    (
        6,
        "create_checkpoints",
        include_str!("../../migrations/V006__create_checkpoints.sql"),
    ),
    (
        7,
        "create_mailbox",
        include_str!("../../migrations/V007__create_mailbox.sql"),
    ),
    (
        8,
        "create_tool_invocations",
        include_str!("../../migrations/V008__create_tool_invocations.sql"),
    ),
    (
        9,
        "create_migration_history",
        include_str!("../../migrations/V009__create_migration_history.sql"),
    ),
    (
        10,
        "create_documents",
        include_str!("../../migrations/V010__create_documents.sql"),
    ),
    (
        11,
        "create_chunks",
        include_str!("../../migrations/V011__create_chunks.sql"),
    ),
    (
        12,
        "create_graph_nodes",
        include_str!("../../migrations/V012__create_graph_nodes.sql"),
    ),
    (
        13,
        "create_graph_edges",
        include_str!("../../migrations/V013__create_graph_edges.sql"),
    ),
    (
        14,
        "add_chunks_fts",
        include_str!("../../migrations/V014__add_chunks_fts.sql"),
    ),
    (
        15,
        "add_task_approval_titles",
        include_str!("../../migrations/V015__add_task_approval_titles.sql"),
    ),
];

impl PgMigrationRunner {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Ensure the migration history table exists.
    async fn ensure_history_table(&self) -> Result<(), StoreError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _cairn_migrations (
                version     INTEGER PRIMARY KEY,
                name        TEXT NOT NULL,
                applied_at  BIGINT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Migration(e.to_string()))?;
        Ok(())
    }

    /// List migrations that have already been applied.
    pub async fn applied(&self) -> Result<Vec<AppliedMigration>, StoreError> {
        self.ensure_history_table().await?;

        let rows = sqlx::query_as::<_, (i32, String, i64)>(
            "SELECT version, name, applied_at FROM _cairn_migrations ORDER BY version",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Migration(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(version, name, applied_at)| AppliedMigration {
                version: version as u32,
                name,
                applied_at: applied_at as u64,
            })
            .collect())
    }

    /// Apply all pending migrations in version order within a transaction.
    pub async fn run_pending(&self) -> Result<Vec<AppliedMigration>, StoreError> {
        self.ensure_history_table().await?;

        let applied = self.applied().await?;
        let max_applied = applied.iter().map(|m| m.version).max().unwrap_or(0);

        let pending: Vec<_> = MIGRATIONS
            .iter()
            .filter(|(v, _, _)| *v > max_applied)
            .collect();

        if pending.is_empty() {
            return Ok(vec![]);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::Migration(e.to_string()))?;

        let mut results = Vec::new();

        for (version, name, sql) in &pending {
            // Execute the migration SQL (may contain multiple statements).
            for statement in sql.split(';').filter(|s| !s.trim().is_empty()) {
                sqlx::query(statement.trim())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| StoreError::Migration(format!("V{version:03}__{name}: {e}")))?;
            }

            // Record in history.
            sqlx::query(
                "INSERT INTO _cairn_migrations (version, name, applied_at) VALUES ($1, $2, $3)",
            )
            .bind(*version as i32)
            .bind(*name)
            .bind(now as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| StoreError::Migration(e.to_string()))?;

            results.push(AppliedMigration {
                version: *version,
                name: name.to_string(),
                applied_at: now,
            });
        }

        tx.commit()
            .await
            .map_err(|e| StoreError::Migration(e.to_string()))?;

        Ok(results)
    }
}
