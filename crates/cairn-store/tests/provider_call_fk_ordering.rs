//! Regression test for F24 dogfood blocker (2026-04-23).
//!
//! When `TracingEmitter::on_decide_completed` in `cairn-app` emits
//! `ProviderCallCompleted` against the durable secondary (Postgres in
//! `--mode team`), the `provider_calls.route_decision_id` FK refers
//! to `route_decisions(route_decision_id)`. The fix requires the
//! emitter to append `RouteDecisionMade` BEFORE the
//! `ProviderCallCompleted` event, in a single `append(&[...])` call,
//! so the sync projection processes them in order within one
//! transaction.
//!
//! Dogfood symptom (pre-fix):
//!   ```
//!   ERROR: secondary event log write failed — in-memory log has 1
//!   event(s) the secondary did not commit. error: internal store
//!   error: error returned from database: insert or update on table
//!   "provider_calls" violates foreign key constraint
//!   "provider_calls_route_decision_id_fkey"
//!   ```
//!
//! The workspace tests don't stand up a live Postgres instance, so
//! this file asserts the invariant against the SQLite backend (which
//! enforces the same FK — `PRAGMA foreign_keys=ON` — with the
//! identical schema mirrored in `crates/cairn-store/src/sqlite/schema.rs`).
//! A separate static-SQL check covers the Postgres migration.

use cairn_domain::providers::{
    OperationKind, ProviderCallStatus, RouteDecisionStatus,
};
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, ProviderBindingId,
    ProviderCallCompleted, ProviderCallId, ProviderConnectionId, ProviderModelId,
    RouteAttemptId, RouteDecisionId, RouteDecisionMade, RuntimeEvent, TenantId,
    WorkspaceId,
};

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_fk"),
        workspace_id: WorkspaceId::new("w_fk"),
        project_id: ProjectId::new("p_fk"),
    }
}

fn envelope(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn route_made(decision_id: &str, binding_id: &str, decided_at: u64) -> RuntimeEvent {
    RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
        project: project(),
        route_decision_id: RouteDecisionId::new(decision_id),
        operation_kind: OperationKind::Generate,
        selected_provider_binding_id: Some(ProviderBindingId::new(binding_id)),
        final_status: RouteDecisionStatus::Selected,
        attempt_count: 1,
        fallback_used: false,
        decided_at,
    })
}

fn provider_call(
    call_id: &str,
    decision_id: &str,
    attempt_id: &str,
    binding_id: &str,
    completed_at: u64,
) -> RuntimeEvent {
    RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
        project: project(),
        provider_call_id: ProviderCallId::new(call_id),
        route_decision_id: RouteDecisionId::new(decision_id),
        route_attempt_id: RouteAttemptId::new(attempt_id),
        provider_binding_id: ProviderBindingId::new(binding_id),
        provider_connection_id: ProviderConnectionId::new(binding_id),
        provider_model_id: ProviderModelId::new("glm-4.7"),
        operation_kind: OperationKind::Generate,
        status: ProviderCallStatus::Succeeded,
        latency_ms: Some(42),
        input_tokens: Some(100),
        output_tokens: Some(50),
        cost_micros: Some(1_000),
        completed_at,
        session_id: None,
        run_id: None,
        error_class: None,
        raw_error_message: None,
        retry_count: 0,
        task_id: None,
        prompt_release_id: None,
        fallback_position: 0,
        started_at: completed_at.saturating_sub(42),
        finished_at: completed_at,
    })
}

// ── SQLite: live FK-enforced projection tests ─────────────────────────

#[cfg(feature = "sqlite")]
mod sqlite {
    use super::*;
    use cairn_store::sqlite::{SqliteAdapter, SqliteEventLog};
    use cairn_store::EventLog;

    async fn open_log() -> SqliteEventLog {
        let adapter = SqliteAdapter::in_memory()
            .await
            .expect("sqlite in-memory must open");
        SqliteEventLog::new(adapter.pool().clone())
    }

    /// F24 repro + fix: RouteDecisionMade emitted BEFORE
    /// ProviderCallCompleted in a single append must land both rows
    /// (FK satisfied). This is the shape that `TracingEmitter` in
    /// `cairn-app` now uses.
    #[tokio::test]
    async fn route_decision_made_before_provider_call_satisfies_fk() {
        let log = open_log().await;
        let now = 1_700_000_000_000u64;

        log.append(&[
            envelope("e1_route", route_made("rd_ok", "binding_brain", now)),
            envelope(
                "e2_call",
                provider_call("call_ok", "rd_ok", "ra_ok", "binding_brain", now + 100),
            ),
        ])
        .await
        .expect("ordered append must succeed under FK");
    }

    /// Regression guard: flipping the order must fail. The in-order
    /// emit is the *only* thing keeping us FK-clean on Postgres; if a
    /// future refactor accidentally reverses the slice or splits it
    /// across two appends, this test fires. The ProviderCallCompleted
    /// row references a decision_id that has not yet been projected
    /// when the provider_calls INSERT runs.
    ///
    /// Note: SQLite applies the projection one event at a time within
    /// the same transaction (mirroring PgEventLog::append). Emitting
    /// the call before the decision therefore fails inside the INSERT
    /// rather than at COMMIT time.
    #[tokio::test]
    async fn reversed_order_violates_fk() {
        let log = open_log().await;
        let now = 1_700_000_000_000u64;

        let err = log
            .append(&[
                envelope(
                    "e1_call",
                    provider_call(
                        "call_bad",
                        "rd_bad",
                        "ra_bad",
                        "binding_brain",
                        now + 100,
                    ),
                ),
                envelope("e2_route", route_made("rd_bad", "binding_brain", now)),
            ])
            .await
            .expect_err("reversed order must violate FK");

        let msg = err.to_string().to_ascii_lowercase();
        assert!(
            msg.contains("foreign key") || msg.contains("constraint"),
            "expected FK violation error, got: {msg}"
        );
    }

    /// Splitting into two appends was the pre-F24 shape (caller was
    /// expected to emit decisions out-of-band but the
    /// `on_decide_completed` hook only emitted the provider call).
    /// The missing-parent case also fails, which is the actual F24
    /// production symptom.
    #[tokio::test]
    async fn provider_call_without_route_decision_violates_fk() {
        let log = open_log().await;
        let now = 1_700_000_000_000u64;

        let err = log
            .append(&[envelope(
                "e_solo_call",
                provider_call(
                    "call_solo",
                    "rd_never_emitted",
                    "ra_solo",
                    "binding_brain",
                    now,
                ),
            )])
            .await
            .expect_err("provider_call with no parent decision must violate FK");

        let msg = err.to_string().to_ascii_lowercase();
        assert!(
            msg.contains("foreign key") || msg.contains("constraint"),
            "expected FK violation error, got: {msg}"
        );
    }
}

// ── Postgres: static schema/migration assertions ──────────────────────
//
// The workspace doesn't stand up live Postgres in tests. We assert the
// migration text still declares the FK so operators can't accidentally
// drop it and reintroduce the dogfood symptom. If we ever migrate to
// DEFERRABLE or drop the FK as an application-level invariant, the
// assertions here must be updated in the same PR — the comment trail
// will make the rationale obvious to reviewers.

#[cfg(feature = "postgres")]
mod postgres {
    use cairn_store::pg::registered_migrations;

    fn v016_sql() -> &'static str {
        registered_migrations()
            .iter()
            .find(|(v, _, _)| *v == 16)
            .map(|(_, _, sql)| *sql)
            .expect("V016 create_prompt_and_routing_state must be registered")
    }

    #[test]
    fn v016_declares_provider_calls_fk_to_route_decisions() {
        let sql = v016_sql().to_ascii_lowercase();
        // FK is expressed as `REFERENCES route_decisions(route_decision_id)`.
        assert!(
            sql.contains("references route_decisions(route_decision_id)")
                || sql.contains("references route_decisions (route_decision_id)"),
            "V016 must keep FK provider_calls.route_decision_id -> route_decisions.route_decision_id"
        );
    }

    #[test]
    fn v016_declares_provider_calls_table() {
        let sql = v016_sql().to_ascii_lowercase();
        assert!(
            sql.contains("create table if not exists provider_calls"),
            "V016 must create provider_calls"
        );
        assert!(
            sql.contains("create table if not exists route_decisions"),
            "V016 must create route_decisions"
        );
    }
}
