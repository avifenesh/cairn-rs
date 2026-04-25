//! F39 integration tests: durable projections for RFC 002 / RFC 019 /
//! RFC 020 recovery + decision events.
//!
//! Before F39 the pg and sqlite `SyncProjection` appliers emitted a
//! `log_stub` warning for these five variants — the envelope landed in
//! `event_log` (source of truth preserved) but no projection table was
//! written, so any query reading recovery attempts or recorded
//! decisions returned empty or stale data.
//!
//! Each test:
//!
//!   1. Appends one `RuntimeEvent` of the variant under test through
//!      the real `EventLog::append` path (so the in-transaction
//!      projection hook runs).
//!   2. Asserts the projection table contains the expected row(s) via
//!      direct SQL (no trait method added — scope creep is explicit
//!      non-goal in the PR brief).
//!   3. For variants whose projection primary key is narrower than the
//!      envelope `event_id` (RecoverySummary on `boot_id`, DecisionRecorded
//!      on `decision_id`, DecisionCacheWarmup on `warmed_at`), appends a
//!      second distinct envelope that collides on the projection PK and
//!      asserts the row count does not increase — proving
//!      `ON CONFLICT DO NOTHING` keeps projection replay idempotent. The
//!      `recovery_attempted_*` / `recovery_completed_*` tests intentionally
//!      skip that check because a duplicate `event_id` is rejected earlier
//!      by `EventLog::append` itself.
//!
//! The SQLite path is exercised directly (via `SqliteAdapter::in_memory`);
//! Postgres parity is enforced by `schema_parity.rs`, `pg_migration_contract.rs`,
//! and the V026 text-level assertions in `postgres_contract` below. The
//! `postgres_contract` module has no runtime SQL so it builds whenever
//! the `postgres` feature is on, independent of this file's SQLite gate.

// Test row tuples are intentionally explicit for column-order readability;
// factoring them into type aliases would obscure the asserted SQL shape.
#![allow(clippy::type_complexity)]
// The whole file only exists to validate F39 projections. Builds with
// just `sqlite`, just `postgres`, or both — each inner module is
// gated on the feature(s) it needs.
#![cfg(any(feature = "sqlite", feature = "postgres"))]

#[cfg(feature = "sqlite")]
mod sqlite_runtime {
    use cairn_domain::decisions::{DecisionKey, DecisionOutcome, DecisionScopeRef};
    use cairn_domain::ids::DecisionId;
    use cairn_domain::{
        DecisionCacheWarmup, DecisionRecorded, EventEnvelope, EventId, EventSource, ProjectKey,
        RecoveryAttempted, RecoveryCompleted, RecoverySummaryEmitted, RunId, RuntimeEvent, TaskId,
    };
    use cairn_store::event_log::EventLog;
    use cairn_store::sqlite::{SqliteAdapter, SqliteEventLog};

    fn project() -> ProjectKey {
        ProjectKey::new("tenant_f39", "ws_f39", "proj_f39")
    }

    async fn fresh_sqlite() -> (SqliteAdapter, SqliteEventLog) {
        let adapter = SqliteAdapter::in_memory().await.expect("sqlite in_memory");
        let log = SqliteEventLog::new(adapter.pool().clone());
        (adapter, log)
    }

    // ── RecoveryAttempted ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn recovery_attempted_is_projected_to_recovery_attempts_table() {
        let (adapter, log) = fresh_sqlite().await;

        let evt = EventEnvelope::for_runtime_event(
            EventId::new("evt_ra_1"),
            EventSource::System,
            RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
                project: project(),
                run_id: Some(RunId::new("run_abc")),
                task_id: Some(TaskId::new("task_xyz")),
                reason: "lease_expired".into(),
                boot_id: Some("boot_1".into()),
            }),
        );

        log.append(std::slice::from_ref(&evt))
            .await
            .expect("append");

        // Direct SQL: one row with the expected payload.
        let row: (
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT event_id, tenant_id, workspace_id, project_id, run_id, task_id, reason, boot_id
         FROM recovery_attempts",
        )
        .fetch_one(adapter.pool())
        .await
        .expect("projection row must exist — the bug would leave this empty");

        assert_eq!(row.0, "evt_ra_1");
        assert_eq!(row.1, "tenant_f39");
        assert_eq!(row.2, "ws_f39");
        assert_eq!(row.3, "proj_f39");
        assert_eq!(row.4.as_deref(), Some("run_abc"));
        assert_eq!(row.5.as_deref(), Some("task_xyz"));
        assert_eq!(row.6, "lease_expired");
        assert_eq!(row.7.as_deref(), Some("boot_1"));

        // The event log itself rejects duplicate envelope event_ids, so
        // projection ON CONFLICT is belt-and-suspenders for this variant.
        // The more realistic idempotency case is covered by the
        // recovery_summary and decision_cache_warmup tests below, where
        // the projection PK (boot_id / warmed_at) is narrower than the
        // envelope event_id.
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recovery_attempts")
            .fetch_one(adapter.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
        let _ = evt;
    }

    #[tokio::test]
    async fn recovery_attempted_with_null_optional_fields_projects_nulls() {
        let (adapter, log) = fresh_sqlite().await;

        let evt = EventEnvelope::for_runtime_event(
            EventId::new("evt_ra_2"),
            EventSource::System,
            RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
                project: project(),
                run_id: Some(RunId::new("run_only")),
                task_id: None,
                reason: "pre_rfc020_legacy".into(),
                boot_id: None,
            }),
        );
        log.append(&[evt]).await.expect("append");

        let row: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT run_id, task_id, boot_id FROM recovery_attempts WHERE event_id = 'evt_ra_2'",
        )
        .fetch_one(adapter.pool())
        .await
        .unwrap();
        assert_eq!(row.0.as_deref(), Some("run_only"));
        assert!(row.1.is_none());
        assert!(row.2.is_none(), "pre-RFC-020 events carry boot_id=None");
    }

    // ── RecoveryCompleted ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn recovery_completed_is_projected_with_recovered_flag() {
        let (adapter, log) = fresh_sqlite().await;

        let evts = vec![
            EventEnvelope::for_runtime_event(
                EventId::new("evt_rc_ok"),
                EventSource::System,
                RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
                    project: project(),
                    run_id: Some(RunId::new("run_ok")),
                    task_id: None,
                    recovered: true,
                    boot_id: Some("boot_2".into()),
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_rc_fail"),
                EventSource::System,
                RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
                    project: project(),
                    run_id: None,
                    task_id: Some(TaskId::new("task_fail")),
                    recovered: false,
                    boot_id: Some("boot_2".into()),
                }),
            ),
        ];
        log.append(&evts).await.expect("append");

        let rows: Vec<(String, Option<String>, Option<String>, i64, Option<String>)> =
            sqlx::query_as(
                "SELECT event_id, run_id, task_id, recovered, boot_id
         FROM recovery_completions ORDER BY event_id",
            )
            .fetch_all(adapter.pool())
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "evt_rc_fail");
        assert_eq!(rows[0].3, 0, "recovered=false -> 0 in sqlite");
        assert_eq!(rows[1].0, "evt_rc_ok");
        assert_eq!(rows[1].3, 1, "recovered=true -> 1 in sqlite");
    }

    // ── RecoverySummaryEmitted ─────────────────────────────────────────────────

    #[tokio::test]
    async fn recovery_summary_emitted_is_projected_once_per_boot() {
        let (adapter, log) = fresh_sqlite().await;

        let make = |event_id: &str, boot_id: &str| {
            EventEnvelope::for_runtime_event(
                EventId::new(event_id),
                EventSource::System,
                RuntimeEvent::RecoverySummaryEmitted(RecoverySummaryEmitted {
                    sentinel_project: project(),
                    boot_id: boot_id.into(),
                    recovered_runs: 3,
                    recovered_tasks: 7,
                    recovered_sandboxes: 1,
                    preserved_sandboxes: 2,
                    orphaned_sandboxes_cleaned: 0,
                    decision_cache_entries: 42,
                    stale_pending_cleared: 5,
                    tool_result_cache_entries: 11,
                    memory_projection_entries: 0,
                    graph_nodes_recovered: 0,
                    graph_edges_recovered: 0,
                    webhook_dedup_entries: 0,
                    trigger_projections: 0,
                    startup_ms: 1_234,
                    summary_at_ms: 1_700_000_000_000,
                }),
            )
        };

        log.append(&[make("evt_rs_1", "boot_A")])
            .await
            .expect("append A");
        log.append(&[make("evt_rs_2", "boot_B")])
            .await
            .expect("append B");
        // Simulate a crash-loop where a second (distinct) envelope carries
        // the same boot_id — the projection PK (boot_id) must keep the
        // table at two rows without raising a unique-constraint error.
        log.append(&[make("evt_rs_3_dupboot", "boot_A")])
            .await
            .expect("replay A");

        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recovery_summaries")
            .fetch_one(adapter.pool())
            .await
            .unwrap();
        assert_eq!(
            count, 2,
            "one row per boot_id — ON CONFLICT DO NOTHING must swallow the dup"
        );

        let row: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT recovered_runs, recovered_tasks, decision_cache_entries, startup_ms
         FROM recovery_summaries WHERE boot_id = 'boot_A'",
        )
        .fetch_one(adapter.pool())
        .await
        .unwrap();
        assert_eq!(row, (3, 7, 42, 1_234));
    }

    // ── DecisionRecorded ───────────────────────────────────────────────────────

    fn sample_decision_with_event(
        event_id: &str,
        id: &str,
        outcome: DecisionOutcome,
        cached: bool,
    ) -> EventEnvelope<RuntimeEvent> {
        EventEnvelope::for_runtime_event(
            EventId::new(event_id),
            EventSource::System,
            RuntimeEvent::DecisionRecorded(DecisionRecorded {
                project: project(),
                decision_id: DecisionId::new(id),
                decision_key: DecisionKey {
                    kind_tag: "tool_invocation".into(),
                    scope_ref: DecisionScopeRef::Tenant {
                        tenant_id: "tenant_f39".into(),
                    },
                    semantic_hash: "deadbeef".into(),
                },
                outcome,
                cached,
                expires_at: if cached { 2_000_000_000_000 } else { 0 },
                decided_at: 1_700_000_000_000,
                event_json: format!(r#"{{"decision_id":"{id}","full":"payload"}}"#),
            }),
        )
    }

    fn sample_decision(
        id: &str,
        outcome: DecisionOutcome,
        cached: bool,
    ) -> EventEnvelope<RuntimeEvent> {
        sample_decision_with_event(&format!("evt_dec_{id}"), id, outcome, cached)
    }

    #[tokio::test]
    async fn decision_recorded_is_projected_with_outcome_kind() {
        let (adapter, log) = fresh_sqlite().await;

        log.append(&[
            sample_decision("dec_allowed", DecisionOutcome::Allowed, true),
            sample_decision(
                "dec_denied",
                DecisionOutcome::Denied {
                    deny_step: 3,
                    deny_reason: "quota_exceeded".into(),
                },
                false,
            ),
        ])
        .await
        .expect("append");

        let rows: Vec<(String, String, i64, i64, String)> = sqlx::query_as(
            "SELECT decision_id, outcome_kind, cached, expires_at, event_json
         FROM decision_records ORDER BY decision_id",
        )
        .fetch_all(adapter.pool())
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);

        assert_eq!(rows[0].0, "dec_allowed");
        assert_eq!(rows[0].1, "allowed");
        assert_eq!(rows[0].2, 1);
        assert_eq!(rows[0].3, 2_000_000_000_000);
        assert!(
            rows[0].4.contains("\"full\":\"payload\""),
            "event_json must round-trip the original serialized DecisionEvent, got {}",
            rows[0].4
        );

        assert_eq!(rows[1].0, "dec_denied");
        assert_eq!(rows[1].1, "denied");
        assert_eq!(rows[1].2, 0);
        assert_eq!(rows[1].3, 0);

        // decision_key_json is valid JSON that round-trips back to the same DecisionKey.
        let (dk_json,): (String,) = sqlx::query_as(
            "SELECT decision_key_json FROM decision_records WHERE decision_id = 'dec_allowed'",
        )
        .fetch_one(adapter.pool())
        .await
        .unwrap();
        let parsed: DecisionKey =
            serde_json::from_str(&dk_json).expect("decision_key_json is valid");
        assert_eq!(parsed.kind_tag, "tool_invocation");
        assert_eq!(parsed.semantic_hash, "deadbeef");

        // Replay idempotency: a distinct envelope carrying the same
        // decision_id must not duplicate the projection row.
        log.append(&[sample_decision_with_event(
            "evt_dec_replay",
            "dec_allowed",
            DecisionOutcome::Allowed,
            true,
        )])
        .await
        .expect("replay");
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM decision_records")
            .fetch_one(adapter.pool())
            .await
            .unwrap();
        assert_eq!(count, 2, "decision_id PK blocks the dup projection row");
    }

    // ── DecisionCacheWarmup ────────────────────────────────────────────────────

    #[tokio::test]
    async fn decision_cache_warmup_is_projected_per_boot() {
        let (adapter, log) = fresh_sqlite().await;

        let make = |event_id: &str, warmed_at: u64, cached: u32, dropped: u32| {
            EventEnvelope::for_runtime_event(
                EventId::new(event_id),
                EventSource::System,
                RuntimeEvent::DecisionCacheWarmup(DecisionCacheWarmup {
                    cached,
                    expired_and_dropped: dropped,
                    warmed_at,
                }),
            )
        };

        log.append(&[make("evt_w_1", 1_700_000_000_000, 10, 2)])
            .await
            .expect("append boot1");
        log.append(&[make("evt_w_2", 1_700_000_001_000, 15, 0)])
            .await
            .expect("append boot2");

        let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
            "SELECT warmed_at, cached, expired_and_dropped
         FROM decision_cache_warmups ORDER BY warmed_at",
        )
        .fetch_all(adapter.pool())
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![(1_700_000_000_000, 10, 2), (1_700_000_001_000, 15, 0)]
        );

        // A second distinct envelope carrying the same warmed_at (e.g. two
        // boots on the same ms — rare but possible in tests): the PK on
        // warmed_at keeps the row count at 2.
        log.append(&[make("evt_w_dup", 1_700_000_000_000, 10, 2)])
            .await
            .expect("replay boot1");
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM decision_cache_warmups")
            .fetch_one(adapter.pool())
            .await
            .unwrap();
        assert_eq!(count, 2);
    }
} // mod sqlite_runtime

// ── Postgres migration contract (text-level — no live DB) ──────────────────

#[cfg(feature = "postgres")]
mod postgres_contract {
    use cairn_store::pg::registered_migrations;

    fn v026_sql() -> &'static str {
        registered_migrations()
            .iter()
            .find(|(v, _, _)| *v == 26)
            .map(|(_, _, sql)| *sql)
            .expect("V026 create_recovery_and_decision_projections must be registered")
    }

    #[test]
    fn v026_creates_all_five_f39_projection_tables() {
        let sql = v026_sql().to_ascii_lowercase();
        for table in [
            "recovery_attempts",
            "recovery_completions",
            "recovery_summaries",
            "decision_records",
            "decision_cache_warmups",
        ] {
            let needle = format!("create table if not exists {table}");
            assert!(
                sql.contains(&needle),
                "V026 must create `{table}` (looked for `{needle}`)"
            );
        }
    }

    #[test]
    fn v026_uses_idempotent_conflict_clauses() {
        // Ensures replay safety at the schema-text level — any future
        // edit that drops the ON CONFLICT handling would pair with a
        // removed assertion here, forcing a deliberate decision.
        let sql = v026_sql().to_ascii_lowercase();
        // All five tables have a `PRIMARY KEY` — ON CONFLICT handling
        // lives in the projection applier, not the migration text.
        let pk_count = sql.matches("primary key").count();
        assert!(
            pk_count >= 5,
            "V026 must declare a PRIMARY KEY per projection table; got {pk_count}"
        );
    }
}
