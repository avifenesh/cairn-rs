# STATUS: pg_migration_contract

**Task:** Postgres migration correctness hardening  
**Tests passed:** 10/10 (with --features postgres); 0/0 without (correctly skipped)  
**File:** `crates/cairn-store/tests/pg_migration_contract.rs`

**Code added:**
- `cairn_store::pg::registered_migrations()` — public compile-time accessor for `MIGRATIONS` const
- Exported from `pg/mod.rs`

**Command:** `cargo test -p cairn-store --test pg_migration_contract --features postgres`

Tests:
- `all_v001_to_v017_migrations_registered_in_order`
- `migration_count_matches_expected` (19 total: V001-V019)
- `migration_versions_are_sequential_no_gaps`
- `migration_versions_are_unique`
- `core_table_names_match_inmemory_projection_fields` (8 core tables verified)
- `v016_prompt_routing_creates_prompt_tables`
- `v017_org_hierarchy_creates_tenants_workspaces_projects`
- `every_migration_has_non_empty_sql`
- `migration_names_are_stable_snake_case_identifiers`
- `run_pending_from_scratch_would_apply_all_migrations` (static arithmetic)
