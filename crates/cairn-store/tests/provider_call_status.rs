//! Provider call status tracking tests (RFC 009).
//!
//! Validates that every provider call's full status is durably stored:
//! success/failure/timeout, error class, latency, token counts, cost, and
//! project scoping.
//!
//! Note: ProviderCallReadModel has get() and list_by_decision().
//! Project-scoped queries use InMemoryStore::list_provider_calls_by_project()
//! (a non-trait helper added alongside RFC 009 hardening).
//!
//! Also fixes a projection bug discovered during this task:
//!   error_class was always stored as None — now populated from the event.

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, ProviderBindingId,
    ProviderCallCompleted, ProviderCallId, ProviderConnectionId, ProviderModelId,
    RouteAttemptId, RouteDecisionId, RuntimeEvent, TenantId, WorkspaceId,
};
use cairn_domain::providers::{
    OperationKind, ProviderCallErrorClass, ProviderCallStatus, RouteDecisionStatus,
};
use cairn_store::{
    projections::ProviderCallReadModel,
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project(tenant: &str, proj: &str) -> ProjectKey {
    ProjectKey {
        tenant_id:    TenantId::new(tenant),
        workspace_id: WorkspaceId::new("w_pcs"),
        project_id:   ProjectId::new(proj),
    }
}

fn default_project() -> ProjectKey { project("t_pcs", "p_pcs") }

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn call(
    evt_id:     &str,
    call_id:    &str,
    decision_id: &str,
    model:      &str,
    proj:       ProjectKey,
    status:     ProviderCallStatus,
    latency:    Option<u64>,
    input:      Option<u32>,
    output:     Option<u32>,
    cost:       Option<u64>,
    error_class: Option<ProviderCallErrorClass>,
    ts:         u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(evt_id, RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
        project:                proj,
        provider_call_id:       ProviderCallId::new(call_id),
        route_decision_id:      RouteDecisionId::new(decision_id),
        route_attempt_id:       RouteAttemptId::new(format!("ra_{call_id}")),
        provider_binding_id:    ProviderBindingId::new("pb_1"),
        provider_connection_id: ProviderConnectionId::new("conn_1"),
        provider_model_id:      ProviderModelId::new(model),
        operation_kind:         OperationKind::Generate,
        status,
        latency_ms:     latency,
        input_tokens:   input,
        output_tokens:  output,
        cost_micros:    cost,
        completed_at:   ts,
        session_id:     None,
        run_id:         None,
        error_class,
        raw_error_message: None,
        retry_count:    0,
    }))
}

// ── 1. ProviderCallCompleted with status=Succeeded ────────────────────────────

#[tokio::test]
async fn succeeded_call_stores_all_fields() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let call_id = ProviderCallId::new("call_ok");

    store.append(&[call(
        "e1", "call_ok", "rd_ok", "gpt-4o", default_project(),
        ProviderCallStatus::Succeeded,
        Some(145), Some(512), Some(256), Some(8_500),
        None, ts,
    )]).await.unwrap();

    let record = ProviderCallReadModel::get(&store, &call_id)
        .await.unwrap()
        .expect("ProviderCallRecord must exist after ProviderCallCompleted");

    assert_eq!(record.status, ProviderCallStatus::Succeeded);
    assert_eq!(record.provider_model_id.as_str(), "gpt-4o");
    assert_eq!(record.latency_ms,    Some(145));
    assert_eq!(record.input_tokens,  Some(512));
    assert_eq!(record.output_tokens, Some(256));
    assert_eq!(record.cost_micros,   Some(8_500));
    assert!(record.error_class.is_none(), "succeeded call has no error class");
}

// ── 2. status=Failed with error class ─────────────────────────────────────────

#[tokio::test]
async fn failed_call_stores_error_class() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[call(
        "e1", "call_fail", "rd_fail", "gpt-4o-mini", default_project(),
        ProviderCallStatus::Failed,
        Some(5_000), None, None, None,
        Some(ProviderCallErrorClass::ProviderError),
        ts,
    )]).await.unwrap();

    let record = ProviderCallReadModel::get(&store, &ProviderCallId::new("call_fail"))
        .await.unwrap().unwrap();

    assert_eq!(record.status, ProviderCallStatus::Failed);
    assert_eq!(record.error_class, Some(ProviderCallErrorClass::ProviderError),
        "error_class must be stored from event (was always None before fix)");
    assert!(record.cost_micros.is_none(), "failed call has no cost");
    assert!(record.input_tokens.is_none());
}

// ── 3. status=Timeout ─────────────────────────────────────────────────────────

#[tokio::test]
async fn timed_out_call_stores_timeout_error_class() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[call(
        "e1", "call_timeout", "rd_to", "claude-3-haiku-20240307", default_project(),
        ProviderCallStatus::Failed,
        None,   // timeout = no measured latency
        None, None, None,
        Some(ProviderCallErrorClass::TimedOut),
        ts,
    )]).await.unwrap();

    let record = ProviderCallReadModel::get(&store, &ProviderCallId::new("call_timeout"))
        .await.unwrap().unwrap();

    assert_eq!(record.status, ProviderCallStatus::Failed);
    assert_eq!(record.error_class, Some(ProviderCallErrorClass::TimedOut),
        "TimedOut error class must persist");
    assert!(record.latency_ms.is_none(),
        "timed-out call has no measured latency");
}

// ── 4. All ProviderCallErrorClass variants persist ────────────────────────────

#[tokio::test]
async fn all_error_class_variants_persist() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let variants = [
        ("call_ec1", ProviderCallErrorClass::TransportFailure),
        ("call_ec2", ProviderCallErrorClass::TimedOut),
        ("call_ec3", ProviderCallErrorClass::RateLimited),
        ("call_ec4", ProviderCallErrorClass::StructuredOutputInvalid),
        ("call_ec5", ProviderCallErrorClass::ProviderError),
        ("call_ec6", ProviderCallErrorClass::Cancelled),
    ];

    for (i, (call_id, ec)) in variants.iter().enumerate() {
        store.append(&[call(
            &format!("e{i}"), call_id, &format!("rd_ec{i}"),
            "gpt-4o", default_project(),
            ProviderCallStatus::Failed,
            None, None, None, None,
            Some(*ec), ts + i as u64,
        )]).await.unwrap();
    }

    for (call_id, expected_ec) in &variants {
        let record = ProviderCallReadModel::get(&store, &ProviderCallId::new(*call_id))
            .await.unwrap().unwrap();
        assert_eq!(record.error_class, Some(*expected_ec),
            "{call_id}: error_class must persist");
    }
}

// ── 5. list_by_project scoping ────────────────────────────────────────────────

#[tokio::test]
async fn list_by_project_returns_only_project_calls() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let proj_a = project("t_scope", "proj_a");
    let proj_b = project("t_scope", "proj_b");

    store.append(&[
        call("e1", "call_a1", "rd_a1", "gpt-4o",      proj_a.clone(),
            ProviderCallStatus::Succeeded, Some(100), Some(200), Some(100), Some(5_000), None, ts),
        call("e2", "call_a2", "rd_a2", "gpt-4o-mini", proj_a.clone(),
            ProviderCallStatus::Succeeded, Some(80),  Some(150), Some(75),  Some(2_000), None, ts + 1),
        call("e3", "call_b1", "rd_b1", "claude-haiku", proj_b.clone(),
            ProviderCallStatus::Failed, None, None, None, None,
            Some(ProviderCallErrorClass::ProviderError), ts + 2),
    ]).await.unwrap();

    let calls_a = store.list_provider_calls_by_project(&proj_a.project_id);
    assert_eq!(calls_a.len(), 2, "project A has 2 calls");
    assert!(calls_a.iter().all(|c| c.project_id == proj_a.project_id));
    let ids_a: Vec<_> = calls_a.iter().map(|c| c.provider_call_id.as_str()).collect();
    assert!(ids_a.contains(&"call_a1"));
    assert!(ids_a.contains(&"call_a2"));
    assert!(!ids_a.contains(&"call_b1"), "proj_b call must not appear in proj_a");

    let calls_b = store.list_provider_calls_by_project(&proj_b.project_id);
    assert_eq!(calls_b.len(), 1);
    assert_eq!(calls_b[0].provider_call_id.as_str(), "call_b1");
    assert_eq!(calls_b[0].status, ProviderCallStatus::Failed);

    let calls_c = store.list_provider_calls_by_project(&ProjectId::new("unknown_proj"));
    assert!(calls_c.is_empty());
}

// ── 6. latency_ms and token counts preserved ──────────────────────────────────

#[tokio::test]
async fn latency_and_token_counts_preserved() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let cases = [
        ("lc_1", Some(45u64),    Some(128u32),  Some(64u32)),
        ("lc_2", Some(3_200u64), Some(4096u32), Some(2048u32)),
        ("lc_3", Some(18u64),    Some(32u32),   Some(16u32)),
        ("lc_4", None,           None,           None),   // failure — no data
    ];

    for (i, (call_id, latency, input, output)) in cases.iter().enumerate() {
        let status = if latency.is_none() {
            ProviderCallStatus::Failed
        } else {
            ProviderCallStatus::Succeeded
        };
        store.append(&[call(
            &format!("elc{i}"), call_id, &format!("rd_lc{i}"),
            "gpt-4o", default_project(), status,
            *latency, *input, *output,
            latency.map(|l| l * 50), // synthetic cost based on latency
            None, ts + i as u64,
        )]).await.unwrap();
    }

    for (call_id, expected_lat, expected_in, expected_out) in &cases {
        let r = ProviderCallReadModel::get(&store, &ProviderCallId::new(*call_id))
            .await.unwrap().unwrap();
        assert_eq!(r.latency_ms,    *expected_lat, "{call_id}: latency_ms");
        assert_eq!(r.input_tokens,  *expected_in,  "{call_id}: input_tokens");
        assert_eq!(r.output_tokens, *expected_out, "{call_id}: output_tokens");
    }
}

// ── 7. cost_micros aggregation across multiple calls ─────────────────────────

#[tokio::test]
async fn cost_aggregation_across_multiple_calls() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Five calls with distinct costs.
    let costs: &[u64] = &[3_000, 7_500, 1_200, 9_800, 4_500];
    let expected_total: u64 = costs.iter().sum(); // 26_000

    for (i, &cost) in costs.iter().enumerate() {
        store.append(&[call(
            &format!("ecost{i}"), &format!("call_cost_{i:02}"),
            &format!("rd_cost{i}"), "gpt-4o",
            default_project(), ProviderCallStatus::Succeeded,
            Some(100), Some(200), Some(100), Some(cost), None,
            ts + i as u64,
        )]).await.unwrap();
    }

    // Verify each call stores its cost.
    for (i, &cost) in costs.iter().enumerate() {
        let r = ProviderCallReadModel::get(
            &store, &ProviderCallId::new(format!("call_cost_{i:02}")),
        ).await.unwrap().unwrap();
        assert_eq!(r.cost_micros, Some(cost), "call {i}: cost_micros must be {cost}");
    }

    // Aggregate by summing all calls for the project.
    let all_calls = store.list_provider_calls_by_project(&default_project().project_id);
    assert_eq!(all_calls.len(), 5);

    let actual_total: u64 = all_calls.iter()
        .filter_map(|c| c.cost_micros)
        .sum();
    assert_eq!(actual_total, expected_total,
        "sum of cost_micros across all calls must be {expected_total}");
}

// ── 8. Rate-limited call ──────────────────────────────────────────────────────

#[tokio::test]
async fn rate_limited_call_stores_correctly() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[call(
        "e1", "call_rl", "rd_rl", "gpt-4o", default_project(),
        ProviderCallStatus::Failed,
        Some(12), None, None, None,  // rate-limit response is fast
        Some(ProviderCallErrorClass::RateLimited),
        ts,
    )]).await.unwrap();

    let r = ProviderCallReadModel::get(&store, &ProviderCallId::new("call_rl"))
        .await.unwrap().unwrap();
    assert_eq!(r.error_class, Some(ProviderCallErrorClass::RateLimited));
    assert_eq!(r.latency_ms, Some(12), "rate-limited calls still have latency");
}

// ── 9. Cancelled call ─────────────────────────────────────────────────────────

#[tokio::test]
async fn cancelled_call_stores_status_and_error_class() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[call(
        "e1", "call_cancel", "rd_cancel", "claude-haiku", default_project(),
        ProviderCallStatus::Cancelled,
        None, None, None, None,
        Some(ProviderCallErrorClass::Cancelled),
        ts,
    )]).await.unwrap();

    let r = ProviderCallReadModel::get(&store, &ProviderCallId::new("call_cancel"))
        .await.unwrap().unwrap();
    assert_eq!(r.status, ProviderCallStatus::Cancelled);
    assert_eq!(r.error_class, Some(ProviderCallErrorClass::Cancelled));
}

// ── 10. get() returns None for unknown call ID ────────────────────────────────

#[tokio::test]
async fn get_returns_none_for_unknown_call_id() {
    let store = InMemoryStore::new();
    let result = ProviderCallReadModel::get(&store, &ProviderCallId::new("ghost"))
        .await.unwrap();
    assert!(result.is_none());
}
