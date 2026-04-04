//! Integration test: guardrail evaluation matrix.

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, RuntimeEvent, TenantId,
    events::{GuardrailPolicyCreated, GuardrailPolicyEvaluated},
    policy::{GuardrailDecisionKind, GuardrailRule, GuardrailSubjectType},
};
use cairn_evals::EvalRunService;
use cairn_store::{EventLog, InMemoryStore};

fn tenant() -> TenantId {
    TenantId::new("t_guardrail")
}

#[tokio::test]
async fn guardrail_matrix_two_evaluations_half_allowed() {
    let store = Arc::new(InMemoryStore::new());

    // Create the policy and evaluate it twice: once allow, once deny.
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

    let svc = EvalRunService::with_event_log(store.clone() as Arc<dyn cairn_store::EventLog>);
    let matrix = svc.build_guardrail_matrix(&tenant()).await.unwrap();

    assert_eq!(matrix.rows.len(), 1, "one policy → one row");

    let row = &matrix.rows[0];
    assert_eq!(row.policy_id, "policy_grd_1");
    assert_eq!(
        row.total_evaluations, 2,
        "total_evaluations must be 2, got {}",
        row.total_evaluations
    );
    assert_eq!(row.allowed_count, 1, "allowed_count must be 1");
    assert_eq!(row.denied_count, 1,  "denied_count must be 1");
    assert!(
        (row.allow_rate - 0.5).abs() < 1e-9,
        "allow_rate must be 0.5, got {}",
        row.allow_rate
    );
}

#[tokio::test]
async fn guardrail_matrix_multiple_policies_grouped_separately() {
    let store = Arc::new(InMemoryStore::new());

    let events: Vec<EventEnvelope<RuntimeEvent>> = vec![
        // policy_a: 3 allowed
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
            EventId::new("ev2"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
                tenant_id: tenant(),
                policy_id: "policy_a".to_owned(),
                subject_type: GuardrailSubjectType::Tool,
                subject_id: None,
                action: "run".to_owned(),
                decision: GuardrailDecisionKind::Allowed,
                reason: None,
                evaluated_at_ms: 2,
            }),
        ),
        EventEnvelope::for_runtime_event(
            EventId::new("ev3"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
                tenant_id: tenant(),
                policy_id: "policy_a".to_owned(),
                subject_type: GuardrailSubjectType::Tool,
                subject_id: None,
                action: "run".to_owned(),
                decision: GuardrailDecisionKind::Allowed,
                reason: None,
                evaluated_at_ms: 3,
            }),
        ),
        // policy_b: 2 denied
        EventEnvelope::for_runtime_event(
            EventId::new("ev4"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
                tenant_id: tenant(),
                policy_id: "policy_b".to_owned(),
                subject_type: GuardrailSubjectType::Channel,
                subject_id: None,
                action: "send".to_owned(),
                decision: GuardrailDecisionKind::Denied,
                reason: None,
                evaluated_at_ms: 4,
            }),
        ),
        EventEnvelope::for_runtime_event(
            EventId::new("ev5"),
            EventSource::Runtime,
            RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
                tenant_id: tenant(),
                policy_id: "policy_b".to_owned(),
                subject_type: GuardrailSubjectType::Channel,
                subject_id: None,
                action: "send".to_owned(),
                decision: GuardrailDecisionKind::Denied,
                reason: None,
                evaluated_at_ms: 5,
            }),
        ),
    ];

    store.append(&events).await.unwrap();

    let svc = EvalRunService::with_event_log(store.clone() as Arc<dyn cairn_store::EventLog>);
    let matrix = svc.build_guardrail_matrix(&tenant()).await.unwrap();

    assert_eq!(matrix.rows.len(), 2, "two policies → two rows");

    let pa = matrix.rows.iter().find(|r| r.policy_id == "policy_a").unwrap();
    let pb = matrix.rows.iter().find(|r| r.policy_id == "policy_b").unwrap();

    assert_eq!(pa.total_evaluations, 3);
    assert_eq!(pa.allowed_count, 3);
    assert_eq!(pa.denied_count, 0);
    assert!((pa.allow_rate - 1.0).abs() < 1e-9, "policy_a: 100% allowed");

    assert_eq!(pb.total_evaluations, 2);
    assert_eq!(pb.allowed_count, 0);
    assert_eq!(pb.denied_count, 2);
    assert!(pb.allow_rate.abs() < 1e-9, "policy_b: 0% allowed");
}

#[tokio::test]
async fn guardrail_matrix_filters_by_tenant() {
    let store = Arc::new(InMemoryStore::new());

    let other_tenant = TenantId::new("other_tenant");

    let events: Vec<EventEnvelope<RuntimeEvent>> = vec![
        // Own tenant event
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
        // Other tenant event — must NOT appear in matrix
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

    let svc = EvalRunService::with_event_log(store.clone() as Arc<dyn cairn_store::EventLog>);
    let matrix = svc.build_guardrail_matrix(&tenant()).await.unwrap();

    assert_eq!(matrix.rows.len(), 1, "only own-tenant events appear");
    assert_eq!(matrix.rows[0].policy_id, "own_policy");
}

#[tokio::test]
async fn guardrail_matrix_empty_when_no_event_log() {
    let svc = EvalRunService::new();
    let matrix = svc.build_guardrail_matrix(&tenant()).await.unwrap();
    assert!(matrix.rows.is_empty(), "no event_log → empty matrix");
}
