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
    (
        16,
        "create_prompt_and_routing_state",
        include_str!("../../migrations/V016__create_prompt_and_routing_state.sql"),
    ),
    (
        17,
        "create_org_hierarchy",
        include_str!("../../migrations/V017__create_org_hierarchy.sql"),
    ),
    (
        18,
        "create_route_policies",
        include_str!("migrations/V018__create_route_policies.sql"),
    ),
    (
        19,
        "create_workspace_members",
        include_str!("migrations/V019__create_workspace_members.sql"),
    ),
    (
        20,
        "add_checkpoint_data_json",
        include_str!("migrations/V020__add_checkpoint_data_json.sql"),
    ),
    (
        21,
        "add_task_session_id",
        include_str!("migrations/V021__add_task_session_id.sql"),
    ),
    (
        22,
        "create_ff_lease_history_cursors",
        include_str!("migrations/V022__create_ff_lease_history_cursors.sql"),
    ),
    (
        23,
        "harden_prompt_schema",
        include_str!("migrations/V023__harden_prompt_schema.sql"),
    ),
    (
        24,
        "create_tool_call_approvals",
        include_str!("migrations/V024__create_tool_call_approvals.sql"),
    ),
    (
        25,
        "create_cost_projections",
        include_str!("migrations/V025__create_cost_projections.sql"),
    ),
    (
        26,
        "create_recovery_and_decision_projections",
        include_str!("migrations/V026__create_recovery_and_decision_projections.sql"),
    ),
];

/// Return the compile-time migration registry as (version, name, sql) triples.
///
/// Used by contract tests to validate the registry without a live database.
pub fn registered_migrations() -> &'static [(u32, &'static str, &'static str)] {
    MIGRATIONS
}

/// Split a SQL script into individual statements, correctly handling:
///
/// - `$$`-dollar-quoted blocks (PL/pgSQL function bodies, etc.)
/// - single-line `--` comments
/// - block `/* … */` comments
///
/// A `;` that appears inside a dollar-quoted block is treated as part of the
/// block rather than a statement terminator.  This avoids the bug where naive
/// `split(';')` breaks `CREATE FUNCTION … AS $$ … ; … $$`.
fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Dollar-quoted string: $tag$ … $tag$
        // Detect opening $…$ tag.
        if chars[i] == '$' {
            // Collect the full dollar-quote tag (e.g. "$$" or "$body$").
            let tag_start = i;
            let mut j = i + 1;
            while j < len && chars[j] != '$' {
                j += 1;
            }
            if j < len {
                // We found a closing '$' for the tag.
                let tag: String = chars[tag_start..=j].iter().collect(); // includes both $
                                                                         // Now scan forward to find the matching closing tag.
                let closing_start = j + 1;
                let tag_chars: Vec<char> = tag.chars().collect();
                let tag_len = tag_chars.len();
                let mut k = closing_start;
                let mut found_close = false;
                while k + tag_len <= len {
                    let slice: Vec<char> = chars[k..k + tag_len].to_vec();
                    if slice == tag_chars {
                        // Found closing tag — include everything up to and
                        // including the closing $tag$ in `current`.
                        let block: String = chars[tag_start..k + tag_len].iter().collect();
                        current.push_str(&block);
                        i = k + tag_len;
                        found_close = true;
                        break;
                    }
                    k += 1;
                }
                if found_close {
                    continue;
                }
                // No closing tag found — treat '$' as a literal character.
            }
            current.push(chars[i]);
            i += 1;
            continue;
        }

        // Single-line comment: -- … \n  — skip entirely, don't add to current.
        if i + 1 < len && chars[i] == '-' && chars[i + 1] == '-' {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // Block comment: /* … */  — skip entirely.
        if i + 1 < len && chars[i] == '/' && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 < len {
                i += 2; // skip closing */
            }
            continue;
        }

        // Statement terminator.
        if chars[i] == ';' {
            let stmt = current.trim().to_owned();
            if !stmt.is_empty() {
                statements.push(stmt);
            }
            current.clear();
            i += 1;
            continue;
        }

        current.push(chars[i]);
        i += 1;
    }

    // Trailing statement without a final semicolon.
    let stmt = current.trim().to_owned();
    if !stmt.is_empty() {
        statements.push(stmt);
    }

    statements
}

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
            // Execute the migration SQL statement by statement.
            // Uses a dollar-quote-aware splitter so that PL/pgSQL function
            // bodies like:
            //   CREATE OR REPLACE FUNCTION ... AS $$
            //   BEGIN ... ; ... END;
            //   $$ LANGUAGE plpgsql;
            // are not incorrectly broken at the semicolons inside $$...$$.
            for statement in split_sql_statements(sql) {
                sqlx::query(&statement)
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

#[cfg(test)]
mod tests {
    use super::split_sql_statements;

    #[test]
    fn simple_statements_are_split() {
        let sql = "CREATE TABLE a (id INT); CREATE TABLE b (id INT);";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("CREATE TABLE a"));
        assert!(stmts[1].starts_with("CREATE TABLE b"));
    }

    #[test]
    fn dollar_quoted_function_is_not_split() {
        let sql = r#"
CREATE OR REPLACE FUNCTION tsv_trigger() RETURNS trigger AS $$
BEGIN
    NEW.tsv := to_tsvector('english', NEW.text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg BEFORE INSERT ON chunks FOR EACH ROW EXECUTE FUNCTION tsv_trigger();
"#;
        let stmts = split_sql_statements(sql);
        // Should be exactly 2: the function and the trigger.
        assert_eq!(stmts.len(), 2, "got: {stmts:?}");
        assert!(
            stmts[0].contains("LANGUAGE plpgsql"),
            "first stmt: {}",
            stmts[0]
        );
        assert!(
            stmts[1].contains("CREATE TRIGGER"),
            "second stmt: {}",
            stmts[1]
        );
    }

    #[test]
    fn single_line_comments_are_handled() {
        let sql = "-- comment\nCREATE TABLE a (id INT); -- trailing\nCREATE TABLE b (id INT);";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(split_sql_statements("").is_empty());
        assert!(split_sql_statements("   -- just a comment\n   ").is_empty());
    }

    #[test]
    fn v014_migration_splits_into_four_statements() {
        // The actual V014 migration content should split into exactly 4 statements:
        // ALTER TABLE, CREATE INDEX, CREATE FUNCTION, CREATE TRIGGER.
        let sql = include_str!("../../migrations/V014__add_chunks_fts.sql");
        let stmts = split_sql_statements(sql);
        assert_eq!(
            stmts.len(),
            4,
            "expected 4 statements, got {}: {stmts:?}",
            stmts.len()
        );
    }
}
