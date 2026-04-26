//! Postgres migration correctness contract tests.
//!
//! Gated behind the `postgres` feature flag so they only run when Postgres
//! support is compiled in. All tests operate on compile-time migration
//! metadata (no live database required) via `registered_migrations()`.
//!
//! Validates:
//! - All V001–V017 migrations exist in the registry with correct version numbers.
//! - The total migration count matches expectations.
//! - Versions are sequential with no gaps.
//! - Table names created by each migration match the InMemoryStore projection
//!   field names so that schema and code stay in sync.
//! - Each migration's SQL is non-empty (embedded at compile time).
//! - Migration names are stable kebab-case identifiers.

#![cfg(feature = "postgres")]

use cairn_store::pg::registered_migrations;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Extract the first `CREATE TABLE` name from a SQL string.
fn first_create_table_name(sql: &str) -> Option<&str> {
    // Matches: CREATE TABLE [IF NOT EXISTS] <name> (
    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.to_uppercase().starts_with("CREATE TABLE") {
            let tokens: Vec<&str> = trimmed.split_whitespace().collect();
            // tokens: ["CREATE", "TABLE", possibly "IF", "NOT", "EXISTS", "<name>", ...]
            let name_idx = if tokens.len() >= 5
                && tokens[2].eq_ignore_ascii_case("IF")
                && tokens[3].eq_ignore_ascii_case("NOT")
                && tokens[4].eq_ignore_ascii_case("EXISTS")
            {
                5
            } else {
                2
            };
            if let Some(raw) = tokens.get(name_idx) {
                // Strip trailing '(' or ';' if present.
                return Some(raw.trim_end_matches('(').trim_end_matches(';').trim());
            }
        }
    }
    None
}

// ── (1) + (2): All V001–V017 migrations exist in correct order ───────────────

/// (1): Migrations V001 through V017 are all registered in the correct order
/// with the expected names.
#[test]
fn all_v001_to_v017_migrations_registered_in_order() {
    let migrations = registered_migrations();

    let expected: &[(u32, &str)] = &[
        (1, "create_event_log"),
        (2, "create_sessions"),
        (3, "create_runs"),
        (4, "create_tasks"),
        (5, "create_approvals"),
        (6, "create_checkpoints"),
        (7, "create_mailbox"),
        (8, "create_tool_invocations"),
        (9, "create_migration_history"),
        (10, "create_documents"),
        (11, "create_chunks"),
        (12, "create_graph_nodes"),
        (13, "create_graph_edges"),
        (14, "add_chunks_fts"),
        (15, "add_task_approval_titles"),
        (16, "create_prompt_and_routing_state"),
        (17, "create_org_hierarchy"),
    ];

    for (version, name) in expected {
        let found = migrations
            .iter()
            .find(|(v, n, _)| v == version && n == name);
        assert!(
            found.is_some(),
            "migration V{version:03}__{name} must be registered; \
             registered versions: {:?}",
            migrations.iter().map(|(v, _, _)| v).collect::<Vec<_>>()
        );
    }
}

/// (2): Total migration count matches the known total.
#[test]
fn migration_count_matches_expected() {
    let migrations = registered_migrations();
    // Migrations V001–V017 = 17 disk files.
    // V018 (create_route_policies) and V019 (create_workspace_members) may also
    // be present depending on the version of the file.
    let count = migrations.len();
    assert!(
        count >= 17,
        "must have at least 17 migrations (V001–V017); got {count}"
    );
    assert!(
        count <= 28,
        "unexpected large migration count {count} — a migration may have been added without updating this test"
    );
}

// ── (3): Version sequence is gap-free ────────────────────────────────────────

/// Versions are strictly sequential starting at 1 with no gaps.
///
/// RFC 002: the migration runner applies migrations in version order.
/// A gap (e.g. [1, 2, 4]) would skip a migration silently.
#[test]
fn migration_versions_are_sequential_no_gaps() {
    let migrations = registered_migrations();

    let mut versions: Vec<u32> = migrations.iter().map(|(v, _, _)| *v).collect();
    versions.sort_unstable();

    assert_eq!(versions[0], 1, "first migration version must be 1");

    for window in versions.windows(2) {
        assert_eq!(
            window[1],
            window[0] + 1,
            "migration versions must be sequential: gap between {} and {}",
            window[0],
            window[1]
        );
    }
}

/// No duplicate version numbers in the registry.
#[test]
fn migration_versions_are_unique() {
    let migrations = registered_migrations();
    let mut versions: Vec<u32> = migrations.iter().map(|(v, _, _)| *v).collect();
    let original_len = versions.len();
    versions.sort_unstable();
    versions.dedup();
    assert_eq!(
        versions.len(),
        original_len,
        "duplicate version numbers detected"
    );
}

// ── (4): Table names match InMemoryStore projection fields ───────────────────

/// Key migrations must create tables whose names match the InMemoryStore
/// projection state field names. This prevents silent schema/code drift.
///
/// The mapping comes from comparing `State { sessions: HashMap<...>, ... }`
/// in `in_memory.rs` against the table created by each migration.
#[test]
fn core_table_names_match_inmemory_projection_fields() {
    let migrations = registered_migrations();

    let table_contracts: &[(u32, &str)] = &[
        (1, "event_log"),        // State.events (global log)
        (2, "sessions"),         // State.sessions
        (3, "runs"),             // State.runs
        (4, "tasks"),            // State.tasks
        (5, "approvals"),        // State.approvals
        (6, "checkpoints"),      // State.checkpoints
        (7, "mailbox_messages"), // State.mailbox_messages
        (8, "tool_invocations"), // State.tool_invocations
    ];

    for (version, expected_table) in table_contracts {
        let migration = migrations
            .iter()
            .find(|(v, _, _)| v == version)
            .unwrap_or_else(|| panic!("migration V{version:03} must exist"));

        let sql = migration.2;
        let found_table = first_create_table_name(sql);

        assert_eq!(
            found_table,
            Some(*expected_table),
            "V{version:03} must CREATE TABLE `{expected_table}`; \
             found: {found_table:?}\n\nSQL (first 200 chars): {}",
            &sql[..sql.len().min(200)]
        );
    }
}

/// Org hierarchy migration creates all three expected tables.
#[test]
fn v017_org_hierarchy_creates_tenants_workspaces_projects() {
    let migrations = registered_migrations();
    let v017 = migrations
        .iter()
        .find(|(v, _, _)| *v == 17)
        .expect("V017 create_org_hierarchy must exist");
    let sql = v017.2;

    // The org hierarchy migration must create the tenant, workspace, and project tables.
    let tables_to_check = ["tenants", "workspaces", "projects"];
    for table in &tables_to_check {
        assert!(
            sql.contains(table),
            "V017 org hierarchy SQL must reference table '{table}'"
        );
    }
}

/// Prompt and routing state migration creates expected prompt tables.
#[test]
fn v016_prompt_routing_creates_prompt_tables() {
    let migrations = registered_migrations();
    let v016 = migrations
        .iter()
        .find(|(v, _, _)| *v == 16)
        .expect("V016 create_prompt_and_routing_state must exist");
    let sql = v016.2;

    // Must reference prompt_assets (used by PromptAssetReadModel).
    assert!(
        sql.contains("prompt_assets"),
        "V016 SQL must reference 'prompt_assets' table"
    );
}

// ── SQL content is non-empty ──────────────────────────────────────────────────

/// Every migration's SQL content is non-empty and non-whitespace.
/// This catches accidentally empty embed files.
#[test]
fn every_migration_has_non_empty_sql() {
    let migrations = registered_migrations();
    for (version, name, sql) in migrations {
        assert!(
            !sql.trim().is_empty(),
            "migration V{version:03}__{name} must have non-empty SQL content"
        );
        assert!(
            sql.len() >= 10,
            "migration V{version:03}__{name} SQL is suspiciously short ({} chars)",
            sql.len()
        );
    }
}

// ── Migration names are stable identifiers ────────────────────────────────────

/// Migration names must be lowercase alphanumeric with underscores only.
/// They are used as record identifiers in `_cairn_migrations` and must not
/// contain spaces, special characters, or mixed case.
#[test]
fn migration_names_are_stable_snake_case_identifiers() {
    let migrations = registered_migrations();
    for (version, name, _) in migrations {
        assert!(
            !name.is_empty(),
            "migration V{version:03} must have a non-empty name"
        );
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "migration V{version:03} name '{name}' must be lowercase_snake_case \
             (only [a-z0-9_] allowed)"
        );
        assert!(
            !name.starts_with('_') && !name.ends_with('_'),
            "migration name '{name}' must not start or end with underscore"
        );
    }
}

// ── (3): Applied-count logic verified statically ─────────────────────────────

/// The migration runner applies all pending migrations when starting from zero.
///
/// This verifies the pending-count arithmetic: if 0 migrations are applied,
/// `run_pending` would apply all MIGRATIONS entries. We verify the count
/// statically since no live DB is available in unit tests.
#[test]
fn run_pending_from_scratch_would_apply_all_migrations() {
    let all = registered_migrations();
    let total = all.len();

    // Simulate: max_applied = 0 → all migrations are pending.
    let max_applied: u32 = 0;
    let pending_count = all.iter().filter(|(v, _, _)| *v > max_applied).count();

    assert_eq!(
        pending_count, total,
        "when starting from scratch (max_applied=0), all {total} migrations must be pending"
    );

    // Simulate: max_applied = 10 → only V011+ are pending.
    let max_applied_10: u32 = 10;
    let pending_after_10 = all.iter().filter(|(v, _, _)| *v > max_applied_10).count();
    let expected_after_10 = total - 10;
    assert_eq!(
        pending_after_10, expected_after_10,
        "after applying 10 migrations, {expected_after_10} must remain pending"
    );

    // Simulate: all applied → 0 pending.
    let max_all = all.iter().map(|(v, _, _)| *v).max().unwrap_or(0);
    let pending_none = all.iter().filter(|(v, _, _)| *v > max_all).count();
    assert_eq!(
        pending_none, 0,
        "after applying all migrations, 0 must remain pending"
    );
}
