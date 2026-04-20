//! Integration test: guardrail evaluation matrix.
//!
//! `build_guardrail_matrix` is currently a stub that always returns an empty
//! `GuardrailMatrix`. These tests verify:
//!   1. The stub returns `Ok` with an empty matrix regardless of input.
//!   2. The `GuardrailPolicyRow` struct fields are accessible and correctly typed.
//!   3. Events can be constructed and appended without error.

use std::sync::Arc;

use cairn_domain::{
    events::{GuardrailPolicyCreated, GuardrailPolicyEvaluated},
    policy::{GuardrailDecisionKind, GuardrailSubjectType},
    EventEnvelope, EventId, EventSource, PolicyId, RuntimeEvent, TenantId,
};
use cairn_evals::matrices::{EvalMetrics, GuardrailMatrix, GuardrailPolicyRow};
use cairn_evals::EvalRunService;
use cairn_store::{EventLog, InMemoryStore};

fn tenant() -> TenantId {
    TenantId::new("t_guardrail")
}

/// Verify that a hand-built `GuardrailPolicyRow` has the expected field types.
#[test]
fn guardrail_policy_row_fields_are_accessible() {
    let row = GuardrailPolicyRow {
        project_id: cairn_domain::ProjectId::new("proj_1"),
        policy_id: PolicyId::new("policy_grd_1"),
        rule_name: "block_tools".to_owned(),
        eval_run_id: cairn_domain::EvalRunId::new("eval_1"),
        metrics: EvalMetrics::default(),
    };

    assert_eq!(row.policy_id, PolicyId::new("policy_grd_1"));
    assert_eq!(row.rule_name, "block_tools");
    assert!(row.metrics.policy_pass_rate.is_none());
}

/// The stub ignores appended events and always returns an empty matrix.
#[tokio::test]
async fn guardrail_matrix_stub_returns_empty_after_events() {
    let store = Arc::new(InMemoryStore::new());

    // Append guardrail events — the stub will not consume them.
    let events: Vec<EventEnvelope<RuntimeEvent>> = vec![
        EventEnvelope::for_runtime_event(
            EventId::new("ev_create"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyCreated(GuardrailPolicyCreated {
                tenant_id: tenant(),
                policy_id: "policy_grd_1".to_owned(),
                name: "Guardrail Test Policy".to_owned(),
                rules: vec![],
            }),
        ),
        EventEnvelope::for_runtime_event(
            EventId::new("ev_allow"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
                tenant_id: tenant(),
                policy_id: "policy_grd_1".to_owned(),
                subject_type: GuardrailSubjectType::Tool,
                subject_id: Some("tool_a".to_owned()),
                action: "invoke".to_owned(),
                decision: GuardrailDecisionKind::Allowed,
                reason: None,
                evaluated_at_ms: 1000,
            }),
        ),
        EventEnvelope::for_runtime_event(
            EventId::new("ev_deny"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
                tenant_id: tenant(),
                policy_id: "policy_grd_1".to_owned(),
                subject_type: GuardrailSubjectType::Tool,
                subject_id: Some("tool_b".to_owned()),
                action: "invoke".to_owned(),
                decision: GuardrailDecisionKind::Denied,
                reason: Some("blocked".to_owned()),
                evaluated_at_ms: 2000,
            }),
        ),
    ];

    store.append(&events).await.unwrap();

    let svc = EvalRunService::with_graph_and_event_log(Arc::new(()), store.clone());
    let matrix = svc.build_guardrail_matrix(&tenant()).await.unwrap();

    // Stub always returns empty; will be populated once event-log projection is wired.
    assert!(
        matrix.rows.is_empty(),
        "stub returns empty matrix (events not yet projected)"
    );
}

/// Multiple policies appended — stub still returns empty matrix.
#[tokio::test]
async fn guardrail_matrix_stub_returns_empty_for_multiple_policies() {
    let store = Arc::new(InMemoryStore::new());

    let events: Vec<EventEnvelope<RuntimeEvent>> = vec![
        EventEnvelope::for_runtime_event(
            EventId::new("ev1"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
                tenant_id: tenant(),
                policy_id: "policy_a".to_owned(),
                subject_type: GuardrailSubjectType::Tool,
                subject_id: None,
                action: "run".to_owned(),
                decision: GuardrailDecisionKind::Allowed,
                reason: None,
                evaluated_at_ms: 1,
            }),
        ),
        EventEnvelope::for_runtime_event(
            EventId::new("ev4"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
                tenant_id: tenant(),
                policy_id: "policy_b".to_owned(),
                subject_type: GuardrailSubjectType::Run,
                subject_id: None,
                action: "send".to_owned(),
                decision: GuardrailDecisionKind::Denied,
                reason: None,
                evaluated_at_ms: 4,
            }),
        ),
    ];

    store.append(&events).await.unwrap();

    let svc = EvalRunService::with_graph_and_event_log(Arc::new(()), store.clone());
    let matrix = svc.build_guardrail_matrix(&tenant()).await.unwrap();

    assert!(
        matrix.rows.is_empty(),
        "stub returns empty matrix regardless of event count"
    );
}

/// Cross-tenant events are appended but stub returns empty for any tenant.
#[tokio::test]
async fn guardrail_matrix_stub_returns_empty_regardless_of_tenant() {
    let store = Arc::new(InMemoryStore::new());

    let other_tenant = TenantId::new("other_tenant");

    let events: Vec<EventEnvelope<RuntimeEvent>> = vec![
        EventEnvelope::for_runtime_event(
            EventId::new("ev_own"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
                tenant_id: tenant(),
                policy_id: "own_policy".to_owned(),
                subject_type: GuardrailSubjectType::Tool,
                subject_id: None,
                action: "run".to_owned(),
                decision: GuardrailDecisionKind::Allowed,
                reason: None,
                evaluated_at_ms: 1,
            }),
        ),
        EventEnvelope::for_runtime_event(
            EventId::new("ev_other"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
                tenant_id: other_tenant,
                policy_id: "other_policy".to_owned(),
                subject_type: GuardrailSubjectType::Tool,
                subject_id: None,
                action: "run".to_owned(),
                decision: GuardrailDecisionKind::Denied,
                reason: None,
                evaluated_at_ms: 2,
            }),
        ),
    ];

    store.append(&events).await.unwrap();

    let svc = EvalRunService::with_graph_and_event_log(Arc::new(()), store.clone());
    let matrix = svc.build_guardrail_matrix(&tenant()).await.unwrap();

    // Stub returns empty; tenant filtering will matter once projection is wired.
    assert!(
        matrix.rows.is_empty(),
        "stub returns empty matrix (tenant filtering not yet implemented)"
    );
}

/// No event log at all — still succeeds with empty matrix.
#[tokio::test]
async fn guardrail_matrix_empty_when_no_event_log() {
    let svc = EvalRunService::new();
    let matrix = svc.build_guardrail_matrix(&tenant()).await.unwrap();
    assert!(matrix.rows.is_empty(), "no event_log → empty matrix");
}

/// `GuardrailMatrix` implements Default with an empty rows vec.
#[test]
fn guardrail_matrix_default_is_empty() {
    let matrix = GuardrailMatrix::default();
    assert!(matrix.rows.is_empty());
}
