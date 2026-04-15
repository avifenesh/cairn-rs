//! Provider call audit trail tests (RFC 009).
//!
//! Validates that every LLM provider call is durably recorded with enough
//! detail for operator audit, cost attribution, and latency analysis.
//!
//! Architecture notes:
//!   ProviderCallReadModel has two methods:
//!     get(call_id)                      — single call lookup
//!     list_by_decision(decision_id)     — all calls for a route decision
//!
//!   ProviderCallRecord.run_id is NOT stored (always None in projection) —
//!   the event's run_id is used to accumulate RunCostRecord instead.
//!   "list_by_run" is therefore implemented via list_by_decision:
//!     one route_decision_id per run → list_by_decision returns the run's calls.
//!
//!   RunCostReadModel accumulates per-run cost from every ProviderCallCompleted
//!   event that carries a run_id.  This is the session-aggregation path.

use cairn_domain::providers::{OperationKind, ProviderCallStatus};
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, ProviderBindingId,
    ProviderCallCompleted, ProviderCallId, ProviderConnectionId, ProviderModelId, RouteAttemptId,
    RouteDecisionId, RunId, RuntimeEvent, SessionId, TenantId, WorkspaceId,
};
use cairn_store::{
    projections::{ProviderCallReadModel, RunCostReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_call"),
        workspace_id: WorkspaceId::new("w_call"),
        project_id: ProjectId::new("p_call"),
    }
}

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Build a ProviderCallCompleted event with all audit fields.
#[allow(clippy::too_many_arguments)]
fn completed_call(
    evt_id: &str,
    call_id: &str,
    decision_id: &str,
    attempt_id: &str,
    model_id: &str,
    run_id: Option<&str>,
    session_id: Option<&str>,
    status: ProviderCallStatus,
    latency_ms: Option<u64>,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    cost_micros: Option<u64>,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
            project: project(),
            provider_call_id: ProviderCallId::new(call_id),
            route_decision_id: RouteDecisionId::new(decision_id),
            route_attempt_id: RouteAttemptId::new(attempt_id),
            provider_binding_id: ProviderBindingId::new("binding_x"),
            provider_connection_id: ProviderConnectionId::new("conn_x"),
            provider_model_id: ProviderModelId::new(model_id),
            operation_kind: OperationKind::Generate,
            status,
            latency_ms,
            input_tokens,
            output_tokens,
            cost_micros,
            completed_at: ts,
            session_id: session_id.map(SessionId::new),
            run_id: run_id.map(RunId::new),
            error_class: None,
            raw_error_message: None,
            retry_count: 0,
            task_id: None,
            prompt_release_id: None,
            fallback_position: 0,
            started_at: 0,
            finished_at: 0,
        }),
    )
}

// ── 1. ProviderCallCompleted stored with all audit fields ─────────────────────

#[tokio::test]
async fn provider_call_completed_stores_all_audit_fields() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let call_id = ProviderCallId::new("call_001");

    store
        .append(&[completed_call(
            "e1",
            "call_001",
            "rd_001",
            "ra_001",
            "gpt-4o",
            Some("run_1"),
            Some("sess_1"),
            ProviderCallStatus::Succeeded,
            Some(145),   // latency_ms
            Some(512),   // input_tokens
            Some(256),   // output_tokens
            Some(8_500), // cost_micros ($0.0085)
            ts,
        )])
        .await
        .unwrap();

    let record = ProviderCallReadModel::get(&store, &call_id)
        .await
        .unwrap()
        .expect("ProviderCallRecord must exist after ProviderCallCompleted");

    assert_eq!(record.provider_call_id, call_id);
    assert_eq!(record.route_decision_id.as_str(), "rd_001");
    assert_eq!(record.provider_model_id.as_str(), "gpt-4o");
    assert_eq!(record.status, ProviderCallStatus::Succeeded);
    assert_eq!(record.latency_ms, Some(145), "latency_ms must be stored");
    assert_eq!(record.input_tokens, Some(512));
    assert_eq!(record.output_tokens, Some(256));
    assert_eq!(
        record.cost_micros,
        Some(8_500),
        "cost_micros must be stored"
    );
}

// ── 2. All ProviderCallStatus variants stored correctly ───────────────────────

#[tokio::test]
async fn provider_call_status_variants_persist() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            completed_call(
                "e1",
                "call_ok",
                "rd_ok",
                "ra_1",
                "gpt-4o",
                None,
                None,
                ProviderCallStatus::Succeeded,
                Some(100),
                Some(256),
                Some(128),
                Some(5_000),
                ts,
            ),
            completed_call(
                "e2",
                "call_fail",
                "rd_fail",
                "ra_2",
                "gpt-4o-mini",
                None,
                None,
                ProviderCallStatus::Failed,
                None,
                None,
                None,
                None,
                ts + 1,
            ),
            completed_call(
                "e3",
                "call_cancel",
                "rd_cancel",
                "ra_3",
                "claude-haiku",
                None,
                None,
                ProviderCallStatus::Cancelled,
                None,
                None,
                None,
                None,
                ts + 2,
            ),
        ])
        .await
        .unwrap();

    let ok = ProviderCallReadModel::get(&store, &ProviderCallId::new("call_ok"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ok.status, ProviderCallStatus::Succeeded);
    assert!(ok.cost_micros.is_some());

    let fail = ProviderCallReadModel::get(&store, &ProviderCallId::new("call_fail"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fail.status, ProviderCallStatus::Failed);
    assert!(fail.cost_micros.is_none(), "failed call has no cost");

    let cancel = ProviderCallReadModel::get(&store, &ProviderCallId::new("call_cancel"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cancel.status, ProviderCallStatus::Cancelled);
}

// ── 3. list_by_decision returns all calls for a route decision ─────────────────
//      (This is the "list_by_run" mechanism: one decision per run → calls scoped)

#[tokio::test]
async fn list_by_decision_returns_all_calls_for_decision() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Two calls for the same route decision (retry scenario).
    store
        .append(&[
            completed_call(
                "e1",
                "call_d1_a",
                "rd_run1",
                "ra_1a",
                "gpt-4o",
                Some("run_1"),
                None,
                ProviderCallStatus::Failed,
                Some(30_000),
                None,
                None,
                None,
                ts,
            ),
            completed_call(
                "e2",
                "call_d1_b",
                "rd_run1",
                "ra_1b",
                "gpt-4o",
                Some("run_1"),
                None,
                ProviderCallStatus::Succeeded,
                Some(145),
                Some(512),
                Some(256),
                Some(8_500),
                ts + 1,
            ),
            // Different decision (different run).
            completed_call(
                "e3",
                "call_d2_a",
                "rd_run2",
                "ra_2a",
                "gpt-4o-mini",
                Some("run_2"),
                None,
                ProviderCallStatus::Succeeded,
                Some(80),
                Some(200),
                Some(100),
                Some(2_000),
                ts + 2,
            ),
        ])
        .await
        .unwrap();

    // list_by_decision("rd_run1") returns both calls for run_1.
    let run1_calls =
        ProviderCallReadModel::list_by_decision(&store, &RouteDecisionId::new("rd_run1"), 10)
            .await
            .unwrap();
    assert_eq!(run1_calls.len(), 2, "two calls for route decision rd_run1");
    let run1_ids: Vec<_> = run1_calls
        .iter()
        .map(|c| c.provider_call_id.as_str())
        .collect();
    assert!(run1_ids.contains(&"call_d1_a"));
    assert!(run1_ids.contains(&"call_d1_b"));

    // list_by_decision("rd_run2") returns only run_2's call.
    let run2_calls =
        ProviderCallReadModel::list_by_decision(&store, &RouteDecisionId::new("rd_run2"), 10)
            .await
            .unwrap();
    assert_eq!(run2_calls.len(), 1);
    assert_eq!(run2_calls[0].provider_call_id.as_str(), "call_d2_a");
}

#[tokio::test]
async fn list_by_decision_limit_is_respected() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    for i in 0u32..5 {
        store
            .append(&[completed_call(
                &format!("e{i}"),
                &format!("call_lim_{i}"),
                "rd_lim",
                &format!("ra_{i}"),
                "gpt-4o",
                None,
                None,
                ProviderCallStatus::Succeeded,
                Some(100 + i as u64 * 10),
                Some(256),
                Some(128),
                Some(5_000),
                ts + i as u64,
            )])
            .await
            .unwrap();
    }

    let limited =
        ProviderCallReadModel::list_by_decision(&store, &RouteDecisionId::new("rd_lim"), 3)
            .await
            .unwrap();
    assert_eq!(limited.len(), 3, "limit=3 caps the results");
}

// ── 4. list_by_session: session aggregation via RunCostReadModel ──────────────

#[tokio::test]
async fn run_cost_accumulates_across_calls_in_same_run() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Three calls for run_1 — costs must accumulate in RunCostRecord.
    store
        .append(&[
            completed_call(
                "e1",
                "c1",
                "rd_1",
                "ra_1",
                "gpt-4o",
                Some("run_acc"),
                None,
                ProviderCallStatus::Succeeded,
                Some(100),
                Some(500),
                Some(250),
                Some(10_000),
                ts,
            ),
            completed_call(
                "e2",
                "c2",
                "rd_2",
                "ra_2",
                "gpt-4o",
                Some("run_acc"),
                None,
                ProviderCallStatus::Succeeded,
                Some(120),
                Some(400),
                Some(200),
                Some(8_000),
                ts + 1,
            ),
            completed_call(
                "e3",
                "c3",
                "rd_3",
                "ra_3",
                "gpt-4o",
                Some("run_acc"),
                None,
                ProviderCallStatus::Succeeded,
                Some(90),
                Some(300),
                Some(150),
                Some(6_000),
                ts + 2,
            ),
        ])
        .await
        .unwrap();

    let run_cost = RunCostReadModel::get_run_cost(&store, &RunId::new("run_acc"))
        .await
        .unwrap()
        .expect("RunCostRecord must exist after ProviderCallCompleted with run_id");

    assert_eq!(run_cost.run_id.as_str(), "run_acc");
    assert_eq!(
        run_cost.total_cost_micros, 24_000,
        "10_000 + 8_000 + 6_000 = 24_000 µUSD"
    );
    assert_eq!(
        run_cost.total_tokens_in, 1_200,
        "500 + 400 + 300 = 1_200 input tokens"
    );
    assert_eq!(
        run_cost.total_tokens_out, 600,
        "250 + 200 + 150 = 600 output tokens"
    );
    assert_eq!(
        run_cost.provider_calls, 3,
        "three provider calls attributed to this run"
    );
}

// ── 5. Session-level aggregation from multiple runs ───────────────────────────

#[tokio::test]
async fn session_aggregates_costs_across_multiple_runs() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // run_a and run_b are in the same session.
    store
        .append(&[
            completed_call(
                "e1",
                "ca1",
                "rda1",
                "ra1",
                "gpt-4o",
                Some("run_a"),
                Some("sess_multi"),
                ProviderCallStatus::Succeeded,
                Some(100),
                Some(400),
                Some(200),
                Some(12_000),
                ts,
            ),
            completed_call(
                "e2",
                "ca2",
                "rda2",
                "ra2",
                "gpt-4o",
                Some("run_a"),
                Some("sess_multi"),
                ProviderCallStatus::Succeeded,
                Some(110),
                Some(300),
                Some(150),
                Some(9_000),
                ts + 1,
            ),
            completed_call(
                "e3",
                "cb1",
                "rdb1",
                "rb1",
                "gpt-4o-mini",
                Some("run_b"),
                Some("sess_multi"),
                ProviderCallStatus::Succeeded,
                Some(80),
                Some(200),
                Some(100),
                Some(3_000),
                ts + 2,
            ),
        ])
        .await
        .unwrap();

    let cost_a = RunCostReadModel::get_run_cost(&store, &RunId::new("run_a"))
        .await
        .unwrap()
        .unwrap();
    let cost_b = RunCostReadModel::get_run_cost(&store, &RunId::new("run_b"))
        .await
        .unwrap()
        .unwrap();

    // Per-run costs are tracked independently.
    assert_eq!(cost_a.total_cost_micros, 21_000, "run_a: 12_000 + 9_000");
    assert_eq!(cost_a.provider_calls, 2);
    assert_eq!(cost_b.total_cost_micros, 3_000);
    assert_eq!(cost_b.provider_calls, 1);

    // Session total = sum of run totals.
    let session_total = cost_a.total_cost_micros + cost_b.total_cost_micros;
    assert_eq!(session_total, 24_000, "session total = run_a + run_b");
    assert_eq!(
        cost_a.total_tokens_in + cost_b.total_tokens_in,
        900,
        "session total input tokens: 400+300+200=900"
    );
}

// ── 6. cost_micros accumulates correctly ─────────────────────────────────────

#[tokio::test]
async fn cost_micros_accumulates_on_each_call() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = RunId::new("run_cost_acc");

    // Append calls one at a time and verify cumulative cost after each.
    for (i, cost) in [5_000u64, 8_000, 3_000, 12_000].iter().enumerate() {
        store
            .append(&[completed_call(
                &format!("e{i}"),
                &format!("call_cost_{i}"),
                &format!("rd_c{i}"),
                &format!("ra_c{i}"),
                "gpt-4o",
                Some("run_cost_acc"),
                None,
                ProviderCallStatus::Succeeded,
                Some(100),
                Some(200),
                Some(100),
                Some(*cost),
                ts + i as u64,
            )])
            .await
            .unwrap();
    }

    let record = RunCostReadModel::get_run_cost(&store, &run_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        record.total_cost_micros, 28_000,
        "5_000 + 8_000 + 3_000 + 12_000 = 28_000 µUSD"
    );
    assert_eq!(record.provider_calls, 4);
}

#[tokio::test]
async fn free_calls_do_not_add_to_cost() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Two calls with cost_micros=0 (free tier / flat-rate model).
    store
        .append(&[
            completed_call(
                "e1",
                "free_1",
                "rd_free",
                "ra_1",
                "llama-free",
                Some("run_free"),
                None,
                ProviderCallStatus::Succeeded,
                Some(50),
                Some(100),
                Some(50),
                Some(0),
                ts,
            ),
            completed_call(
                "e2",
                "free_2",
                "rd_free2",
                "ra_2",
                "llama-free",
                Some("run_free"),
                None,
                ProviderCallStatus::Succeeded,
                Some(45),
                Some(80),
                Some(40),
                Some(0),
                ts + 1,
            ),
        ])
        .await
        .unwrap();

    let cost = RunCostReadModel::get_run_cost(&store, &RunId::new("run_free"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cost.total_cost_micros, 0, "free model adds zero cost");
    assert_eq!(cost.provider_calls, 2, "call count still increments");
    assert_eq!(cost.total_tokens_in, 180, "tokens still accumulate");
}

// ── 7. latency_ms is queryable for percentile calculations ────────────────────

#[tokio::test]
async fn latency_ms_queryable_from_individual_records() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // 5 calls with known latencies for percentile computation.
    let latencies = [50u64, 120, 95, 200, 75];
    for (i, &lat) in latencies.iter().enumerate() {
        store
            .append(&[completed_call(
                &format!("e{i}"),
                &format!("call_lat_{i}"),
                &format!("rd_lat{i}"),
                &format!("ra_{i}"),
                "gpt-4o",
                None,
                None,
                ProviderCallStatus::Succeeded,
                Some(lat),
                Some(200),
                Some(100),
                Some(5_000),
                ts + i as u64,
            )])
            .await
            .unwrap();
    }

    // Retrieve all records and collect latencies.
    let mut measured: Vec<u64> = vec![];
    for i in 0..5 {
        let r = ProviderCallReadModel::get(&store, &ProviderCallId::new(format!("call_lat_{i}")))
            .await
            .unwrap()
            .unwrap();
        if let Some(lat) = r.latency_ms {
            measured.push(lat);
        }
    }

    assert_eq!(measured.len(), 5, "all 5 latencies retrievable");

    // Sort for percentile computation.
    measured.sort_unstable();
    assert_eq!(measured, vec![50, 75, 95, 120, 200]);

    // p50 (median) = index 2 = 95 ms.
    let p50 = measured[measured.len() / 2];
    assert_eq!(p50, 95, "p50 latency = 95ms");

    // p80 = index 3 = 120 ms (80th percentile of 5 samples).
    let p80_idx = (measured.len() as f64 * 0.8).ceil() as usize - 1;
    let p80 = measured[p80_idx];
    assert_eq!(p80, 120, "p80 latency = 120ms");

    // Max (p100) = 200 ms.
    assert_eq!(*measured.last().unwrap(), 200, "max latency = 200ms");
}

#[tokio::test]
async fn failed_calls_without_latency_have_none() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Timed-out call has no latency (connection refused immediately).
    store
        .append(&[completed_call(
            "e1",
            "call_timeout",
            "rd_to",
            "ra_to",
            "gpt-4o",
            None,
            None,
            ProviderCallStatus::Failed,
            None,
            None,
            None,
            None,
            ts,
        )])
        .await
        .unwrap();

    let r = ProviderCallReadModel::get(&store, &ProviderCallId::new("call_timeout"))
        .await
        .unwrap()
        .unwrap();
    assert!(
        r.latency_ms.is_none(),
        "failed call with no timing has None latency"
    );
    assert!(r.input_tokens.is_none());
    assert!(r.cost_micros.is_none());
}

// ── 8. model_id preserved in audit record ────────────────────────────────────

#[tokio::test]
async fn provider_model_id_preserved_in_audit_record() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let models = [
        ("call_m1", "rd_m1", "claude-sonnet-4-6"),
        ("call_m2", "rd_m2", "gpt-4o"),
        ("call_m3", "rd_m3", "meta-llama/llama-3.3-70b-instruct:free"),
    ];

    for (i, (call_id, rd_id, model)) in models.iter().enumerate() {
        store
            .append(&[completed_call(
                &format!("em{i}"),
                call_id,
                rd_id,
                &format!("ra_m{i}"),
                model,
                None,
                None,
                ProviderCallStatus::Succeeded,
                Some(100),
                Some(200),
                Some(100),
                Some(5_000),
                ts + i as u64,
            )])
            .await
            .unwrap();
    }

    for (call_id, _, expected_model) in &models {
        let r = ProviderCallReadModel::get(&store, &ProviderCallId::new(*call_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            r.provider_model_id.as_str(),
            *expected_model,
            "model_id must be preserved in audit record"
        );
    }
}
