//! Prompt schema hardening tests (follow-up to PR #102).
//!
//! Asserts two invariants symmetrically across Postgres and SQLite:
//!
//! 1. `prompt_versions` enforces `UNIQUE(prompt_asset_id, version_number)`.
//! 2. `prompt_assets.updated_at` is `NOT NULL`.
//!
//! The Postgres side is a static check against the embedded migration
//! SQL — the workspace tests don't stand up a live Postgres instance.
//! The SQLite side is a live-DB constraint-violation test using the
//! in-memory adapter.

// ── Postgres: static migration-content assertions ─────────────────────

#[cfg(feature = "postgres")]
mod postgres {
    use cairn_store::pg::registered_migrations;

    fn v023_sql() -> &'static str {
        let migrations = registered_migrations();
        let entry = migrations
            .iter()
            .find(|(v, _, _)| *v == 23)
            .expect("V023 harden_prompt_schema must be registered");
        assert_eq!(entry.1, "harden_prompt_schema", "V023 name must match");
        entry.2
    }

    #[test]
    fn v023_adds_unique_constraint_on_prompt_versions() {
        let sql = v023_sql();
        assert!(
            sql.contains("UNIQUE (prompt_asset_id, version_number)")
                || sql.contains("UNIQUE(prompt_asset_id, version_number)"),
            "V023 must add a UNIQUE(prompt_asset_id, version_number) constraint"
        );
    }

    #[test]
    fn v023_sets_prompt_versions_version_number_not_null() {
        let sql = v023_sql();
        assert!(
            sql.contains("prompt_versions")
                && sql.contains("version_number")
                && sql.contains("SET NOT NULL"),
            "V023 must ALTER prompt_versions.version_number SET NOT NULL"
        );
    }

    #[test]
    fn v023_sets_prompt_assets_updated_at_not_null() {
        let sql = v023_sql();
        // Find the ALTER ... updated_at SET NOT NULL on prompt_assets.
        let lowered = sql.to_ascii_lowercase();
        assert!(
            lowered.contains("alter table prompt_assets")
                && lowered.contains("updated_at")
                && lowered.contains("set not null"),
            "V023 must ALTER prompt_assets.updated_at SET NOT NULL"
        );
    }

    #[test]
    fn v023_backfills_before_constraining() {
        // Constraint application must come *after* the backfill,
        // otherwise the migration would fail on any row that currently
        // violates the invariant. This is a structural check that the
        // UPDATE / DELETE dedup statements appear before the ALTER /
        // ADD CONSTRAINT statements in the migration body.
        let sql = v023_sql();
        let lowered = sql.to_ascii_lowercase();

        let first_backfill = lowered
            .find("update prompt_assets")
            .expect("V023 must contain backfill for prompt_assets");
        let first_alter = lowered
            .find("alter table prompt_assets")
            .expect("V023 must contain ALTER TABLE prompt_assets");
        assert!(
            first_backfill < first_alter,
            "prompt_assets backfill must precede the ALTER TABLE"
        );

        let dedup = lowered
            .find("delete from prompt_versions")
            .expect("V023 must dedup prompt_versions before UNIQUE");
        let add_unique = lowered
            .find("prompt_versions_asset_version_unique")
            .expect("V023 must ADD CONSTRAINT prompt_versions_asset_version_unique");
        assert!(
            dedup < add_unique,
            "prompt_versions dedup must precede the UNIQUE constraint"
        );
    }
}

// ── SQLite: live constraint-violation tests ───────────────────────────

#[cfg(feature = "sqlite")]
mod sqlite {
    use cairn_store::sqlite::SqliteAdapter;

    async fn open() -> SqliteAdapter {
        SqliteAdapter::in_memory()
            .await
            .expect("in-memory SQLite must open")
    }

    /// Seed a prompt_assets row through raw SQL so the test is
    /// independent of the projection implementation. updated_at is
    /// supplied because the hardened schema rejects NULL.
    async fn insert_asset(pool: &sqlx::SqlitePool, asset_id: &str) {
        sqlx::query(
            "INSERT INTO prompt_assets
                 (prompt_asset_id, tenant_id, workspace_id, project_id,
                  name, kind, created_at, updated_at)
             VALUES (?, 't', 'w', 'p', 'n', 'k', 1, 1)",
        )
        .bind(asset_id)
        .execute(pool)
        .await
        .expect("insert prompt_assets");
    }

    #[tokio::test]
    async fn duplicate_asset_version_number_is_rejected() {
        let adapter = open().await;
        let pool = adapter.pool();

        insert_asset(pool, "asset_dup").await;

        // First insert succeeds.
        sqlx::query(
            "INSERT INTO prompt_versions
                 (prompt_version_id, prompt_asset_id, tenant_id, workspace_id, project_id,
                  version_number, content_hash, created_at)
             VALUES ('v1', 'asset_dup', 't', 'w', 'p', 1, 'sha256:a', 1)",
        )
        .execute(pool)
        .await
        .expect("first version insert");

        // Second insert with the same (asset, version_number) must fail.
        let err = sqlx::query(
            "INSERT INTO prompt_versions
                 (prompt_version_id, prompt_asset_id, tenant_id, workspace_id, project_id,
                  version_number, content_hash, created_at)
             VALUES ('v1_dup', 'asset_dup', 't', 'w', 'p', 1, 'sha256:b', 2)",
        )
        .execute(pool)
        .await
        .expect_err("duplicate (prompt_asset_id, version_number) must fail");

        let msg = err.to_string().to_ascii_lowercase();
        assert!(
            msg.contains("unique") || msg.contains("constraint"),
            "expected UNIQUE constraint error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn different_assets_may_reuse_version_number() {
        // Sanity check: the UNIQUE is scoped per asset, not global.
        let adapter = open().await;
        let pool = adapter.pool();

        insert_asset(pool, "asset_a").await;
        insert_asset(pool, "asset_b").await;

        for (vid, aid) in [("va1", "asset_a"), ("vb1", "asset_b")] {
            sqlx::query(
                "INSERT INTO prompt_versions
                     (prompt_version_id, prompt_asset_id, tenant_id, workspace_id, project_id,
                      version_number, content_hash, created_at)
                 VALUES (?, ?, 't', 'w', 'p', 1, 'sha256:x', 1)",
            )
            .bind(vid)
            .bind(aid)
            .execute(pool)
            .await
            .expect("version 1 per asset is allowed");
        }
    }

    #[tokio::test]
    async fn null_version_number_is_rejected() {
        let adapter = open().await;
        let pool = adapter.pool();

        insert_asset(pool, "asset_null_ver").await;

        let err = sqlx::query(
            "INSERT INTO prompt_versions
                 (prompt_version_id, prompt_asset_id, tenant_id, workspace_id, project_id,
                  version_number, content_hash, created_at)
             VALUES ('v_null', 'asset_null_ver', 't', 'w', 'p', NULL, 'sha256:x', 1)",
        )
        .execute(pool)
        .await
        .expect_err("NULL version_number must fail");

        let msg = err.to_string().to_ascii_lowercase();
        assert!(
            msg.contains("not null") || msg.contains("constraint"),
            "expected NOT NULL constraint error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn null_updated_at_on_prompt_assets_is_rejected() {
        let adapter = open().await;
        let pool = adapter.pool();

        let err = sqlx::query(
            "INSERT INTO prompt_assets
                 (prompt_asset_id, tenant_id, workspace_id, project_id,
                  name, kind, created_at, updated_at)
             VALUES ('a_null_updated', 't', 'w', 'p', 'n', 'k', 1, NULL)",
        )
        .execute(pool)
        .await
        .expect_err("NULL updated_at must fail");

        let msg = err.to_string().to_ascii_lowercase();
        assert!(
            msg.contains("not null") || msg.contains("constraint"),
            "expected NOT NULL constraint error, got: {msg}"
        );
    }

    /// End-to-end: the real projection path (PromptVersionCreated) must
    /// still succeed for sequential versions under the hardened schema.
    /// This catches any regression where tightening the schema breaks
    /// the COALESCE(MAX+1) allocator.
    #[tokio::test]
    async fn projection_allocates_sequential_versions_under_hardened_schema() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, PromptAssetCreated,
            PromptAssetId, PromptVersionCreated, PromptVersionId, RuntimeEvent, TenantId,
            WorkspaceId,
        };
        use cairn_store::sqlite::SqliteEventLog;
        use cairn_store::EventLog;

        let adapter = open().await;
        let log = SqliteEventLog::new(adapter.pool().clone());

        let project = ProjectKey {
            tenant_id: TenantId::new("t_h"),
            workspace_id: WorkspaceId::new("w_h"),
            project_id: ProjectId::new("p_h"),
        };
        let asset_id = PromptAssetId::new("asset_hardened");

        let mk_env = |id: &str, ev: RuntimeEvent| {
            EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, ev)
        };

        log.append(&[
            mk_env(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: project.clone(),
                    prompt_asset_id: asset_id.clone(),
                    name: "n".into(),
                    kind: "k".into(),
                    created_at: 1,
                    workspace_id: project.workspace_id.clone(),
                }),
            ),
            mk_env(
                "e2",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: project.clone(),
                    prompt_version_id: PromptVersionId::new("vh1"),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: "sha256:1".into(),
                    created_at: 2,
                    workspace_id: project.workspace_id.clone(),
                }),
            ),
            mk_env(
                "e3",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: project.clone(),
                    prompt_version_id: PromptVersionId::new("vh2"),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: "sha256:2".into(),
                    created_at: 3,
                    workspace_id: project.workspace_id.clone(),
                }),
            ),
        ])
        .await
        .expect("append under hardened schema must succeed");

        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT version_number FROM prompt_versions
              WHERE prompt_asset_id = ?
              ORDER BY version_number",
        )
        .bind(asset_id.as_str())
        .fetch_all(adapter.pool())
        .await
        .unwrap();
        assert_eq!(
            rows.iter().map(|r| r.0).collect::<Vec<_>>(),
            vec![1, 2],
            "projection must allocate version_number 1, 2"
        );
    }
}
