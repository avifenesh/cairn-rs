//! Integration test: skill health matrix aggregates ToolInvocationCompleted/Failed events.

use cairn_domain::{
    EventEnvelope, EventId, EventSource,
    RuntimeEvent, ToolInvocationId,
    tool_invocation::ToolInvocationOutcomeKind,
    events::{ToolInvocationCompleted, ToolInvocationFailed},
    tenancy::ProjectKey,
};
use cairn_evals::EvalRunService;
use cairn_store::{EventLog, InMemoryStore};
use std::sync::Arc;

fn project(tenant_id: &str) -> ProjectKey {
    ProjectKey::new(tenant_id, "workspace", "project")
}

#[tokio::test]
async fn skill_health_matrix_two_complete_one_fail() {
    let store = Arc::new(InMemoryStore::new());
    let tenant_id = cairn_domain::TenantId::new("test_tenant");
    let proj = project("test_tenant");

    let events: Vec<EventEnvelope<RuntimeEvent>> = vec![
        // invocation 1: completed
        EventEnvelope::for_runtime_event(
            EventId::new("evt_1"),
            EventSource::Runtime,
            RuntimeEvent::ToolInvocationCompleted(ToolInvocationCompleted {
                project: proj.clone(),
                invocation_id: ToolInvocationId::new("inv_1"),
                task_id: None,
                tool_name: "tool_a".to_owned(),
                finished_at_ms: 1000,
                outcome: ToolInvocationOutcomeKind::Success,
            }),
        ),
        // invocation 2: completed
        EventEnvelope::for_runtime_event(
            EventId::new("evt_2"),
            EventSource::Runtime,
            RuntimeEvent::ToolInvocationCompleted(ToolInvocationCompleted {
                project: proj.clone(),
                invocation_id: ToolInvocationId::new("inv_2"),
                task_id: None,
                tool_name: "tool_a".to_owned(),
                finished_at_ms: 2000,
                outcome: ToolInvocationOutcomeKind::Success,
            }),
        ),
        // invocation 3: failed
        EventEnvelope::for_runtime_event(
            EventId::new("evt_3"),
            EventSource::Runtime,
            RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
                project: proj.clone(),
                invocation_id: ToolInvocationId::new("inv_3"),
                task_id: None,
                tool_name: "tool_a".to_owned(),
                finished_at_ms: 3000,
                outcome: ToolInvocationOutcomeKind::PermanentFailure,
                error_message: Some("timeout".to_owned()),
            }),
        ),
    ];

    store.append(&events).await.unwrap();

    let svc = EvalRunService::with_event_log(store);
    let matrix = svc.build_skill_health_matrix(&tenant_id).await.unwrap();

    assert_eq!(matrix.rows.len(), 1);
    let row = &matrix.rows[0];
    assert_eq!(row.skill_id, "tool_a");
    assert_eq!(row.invocation_count, 3);
    // 2 out of 3 succeeded: 0.666...
    let success_rate = (row.success_rate * 100.0).round() / 100.0;
    assert_eq!(success_rate, 0.67);
    assert_eq!(row.error_rate + row.success_rate, 1.0);
}
