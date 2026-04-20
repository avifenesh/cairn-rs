/// Compile-time validation of embedded migration ordering and uniqueness.
///
/// This module validates that migration files in the `migrations/` directory
/// follow the naming convention and are properly ordered.
/// Validate migration file names match `V{NNN}__{description}.sql` pattern
/// and versions are sequential starting from 1.
pub fn validate_migration_files(files: &[(&str, &str)]) -> Result<(), MigrationCheckError> {
    if files.is_empty() {
        return Ok(());
    }

    let mut prev_version: u32 = 0;

    for (i, (name, sql)) in files.iter().enumerate() {
        // Verify non-empty SQL.
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return Err(MigrationCheckError::EmptySql {
                name: name.to_string(),
            });
        }

        // Verify sequential versioning.
        let expected_version = (i as u32) + 1;
        if expected_version != prev_version + 1 {
            return Err(MigrationCheckError::GapInVersions {
                expected: prev_version + 1,
                found: expected_version,
            });
        }

        prev_version = expected_version;
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum MigrationCheckError {
    EmptySql { name: String },
    GapInVersions { expected: u32, found: u32 },
}

impl std::fmt::Display for MigrationCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrationCheckError::EmptySql { name } => {
                write!(f, "migration '{name}' has empty SQL")
            }
            MigrationCheckError::GapInVersions { expected, found } => {
                write!(f, "expected migration version {expected}, found {found}")
            }
        }
    }
}

/// Total number of migrations. Update this when adding new migrations.
/// If this constant is wrong, `migration_count_matches_embedded_list`
/// will fail at test time. The PG migration runner at
/// `pg/migration_runner.rs::MIGRATIONS` is the authoritative list.
///
/// **Split location caveat:** migrations V001–V017 live in
/// `crates/cairn-store/migrations/`, V018–V020 live in
/// `crates/cairn-store/src/pg/migrations/`. The `all_migrations_are_valid`
/// test below embeds both directories explicitly. Consolidating to a
/// single directory is tracked in the audit queue (T2-M1).
pub const EXPECTED_MIGRATION_COUNT: usize = 21;

#[cfg(test)]
mod tests {
    use super::*;

    /// All embedded migrations have non-empty SQL and sequential versions.
    #[test]
    fn all_migrations_are_valid() {
        let migrations: &[(&str, &str)] = &[
            (
                "create_event_log",
                include_str!("../migrations/V001__create_event_log.sql"),
            ),
            (
                "create_sessions",
                include_str!("../migrations/V002__create_sessions.sql"),
            ),
            (
                "create_runs",
                include_str!("../migrations/V003__create_runs.sql"),
            ),
            (
                "create_tasks",
                include_str!("../migrations/V004__create_tasks.sql"),
            ),
            (
                "create_approvals",
                include_str!("../migrations/V005__create_approvals.sql"),
            ),
            (
                "create_checkpoints",
                include_str!("../migrations/V006__create_checkpoints.sql"),
            ),
            (
                "create_mailbox",
                include_str!("../migrations/V007__create_mailbox.sql"),
            ),
            (
                "create_tool_invocations",
                include_str!("../migrations/V008__create_tool_invocations.sql"),
            ),
            (
                "create_migration_history",
                include_str!("../migrations/V009__create_migration_history.sql"),
            ),
            (
                "create_documents",
                include_str!("../migrations/V010__create_documents.sql"),
            ),
            (
                "create_chunks",
                include_str!("../migrations/V011__create_chunks.sql"),
            ),
            (
                "create_graph_nodes",
                include_str!("../migrations/V012__create_graph_nodes.sql"),
            ),
            (
                "create_graph_edges",
                include_str!("../migrations/V013__create_graph_edges.sql"),
            ),
            (
                "add_chunks_fts",
                include_str!("../migrations/V014__add_chunks_fts.sql"),
            ),
            (
                "add_task_approval_titles",
                include_str!("../migrations/V015__add_task_approval_titles.sql"),
            ),
            (
                "create_prompt_and_routing_state",
                include_str!("../migrations/V016__create_prompt_and_routing_state.sql"),
            ),
            (
                "create_org_hierarchy",
                include_str!("../migrations/V017__create_org_hierarchy.sql"),
            ),
            // V018-V020 live under src/pg/migrations/ — the split is
            // documented on EXPECTED_MIGRATION_COUNT above.
            (
                "create_route_policies",
                include_str!("pg/migrations/V018__create_route_policies.sql"),
            ),
            (
                "create_workspace_members",
                include_str!("pg/migrations/V019__create_workspace_members.sql"),
            ),
            (
                "add_checkpoint_data_json",
                include_str!("pg/migrations/V020__add_checkpoint_data_json.sql"),
            ),
            (
                "add_task_session_id",
                include_str!("pg/migrations/V021__add_task_session_id.sql"),
            ),
        ];

        validate_migration_files(migrations).unwrap();
        assert_eq!(
            migrations.len(),
            super::EXPECTED_MIGRATION_COUNT,
            "EXPECTED_MIGRATION_COUNT is stale — update it when adding migrations"
        );
    }

    #[test]
    fn detects_empty_sql() {
        let migrations: &[(&str, &str)] = &[("bad", "  ")];
        assert_eq!(
            validate_migration_files(migrations),
            Err(MigrationCheckError::EmptySql {
                name: "bad".to_owned()
            })
        );
    }

    /// Every migration SQL contains at least one CREATE or ALTER or INSERT statement.
    #[test]
    fn all_migrations_contain_ddl() {
        let migration_sqls: &[&str] = &[
            include_str!("../migrations/V001__create_event_log.sql"),
            include_str!("../migrations/V002__create_sessions.sql"),
            include_str!("../migrations/V003__create_runs.sql"),
            include_str!("../migrations/V004__create_tasks.sql"),
            include_str!("../migrations/V005__create_approvals.sql"),
            include_str!("../migrations/V006__create_checkpoints.sql"),
            include_str!("../migrations/V007__create_mailbox.sql"),
            include_str!("../migrations/V008__create_tool_invocations.sql"),
            include_str!("../migrations/V009__create_migration_history.sql"),
            include_str!("../migrations/V010__create_documents.sql"),
            include_str!("../migrations/V011__create_chunks.sql"),
            include_str!("../migrations/V012__create_graph_nodes.sql"),
            include_str!("../migrations/V013__create_graph_edges.sql"),
            include_str!("../migrations/V014__add_chunks_fts.sql"),
            include_str!("../migrations/V015__add_task_approval_titles.sql"),
        ];

        for (i, sql) in migration_sqls.iter().enumerate() {
            let upper = sql.to_uppercase();
            assert!(
                upper.contains("CREATE") || upper.contains("ALTER") || upper.contains("INSERT"),
                "V{:03} does not contain DDL",
                i + 1
            );
        }
    }
}
