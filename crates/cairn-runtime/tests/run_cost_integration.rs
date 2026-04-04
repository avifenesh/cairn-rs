use std::sync::Arc;

use cairn_domain::providers::OperationKind;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, ProviderBindingId, ProviderCallCompleted,
    ProviderCallId, ProviderConnectionId, ProviderModelId, RouteAttemptId, RouteDecisionId, RunId,
    RuntimeEvent, SessionId,
};
use cairn_runtime::services::{RunServiceImpl, SessionServiceImpl};
use cairn_runtime::{RunService, SessionService};
use cairn_store::projections::RunCostReadModel;
use cairn_store::{EventLog, InMemoryStore};

#[tokio::test]
async fn run_cost_accumulates_provider_call_costs() {
    let store = Arc::new(InMemoryStore::new());
    let session_service = SessionServiceImpl::new(store.clone());
    let run_service = RunServiceImpl::new(store.clone());
    let project = ProjectKey::new("tenant_acme", "ws_main", "project_alpha");

    session_service
        .create(&project, SessionId::new("session_cost"))
        .await
        .unwrap();
    run_service
        .start(
            &project,
            &SessionId::new("session_cost"),
            RunId::new("run_cost"),
            None,
        )
        .await
        .unwrap();

    let events = vec![
        EventEnvelope::for_runtime_event(
            EventId::new("evt_provider_call_run_1"),
            EventSource::Runtime,
            RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
                project: project.clone(),
                provider_call_id: ProviderCallId::new("pc_run_cost_1"),
                route_decision_id: RouteDecisionId::new("rd_run_cost_1"),
                route_attempt_id: RouteAttemptId::new("ra_run_cost_1"),
                provider_binding_id: ProviderBindingId::new("binding_cost"),
                provider_connection_id: ProviderConnectionId::new("conn_cost"),
                provider_model_id: ProviderModelId::new("model_cost"),
                run_id: Some(RunId::new("run_cost")),
                operation_kind: OperationKind::Generate,
                status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                latency_ms: Some(10),
                input_tokens: Some(100),
                output_tokens: Some(40),
                cost_micros: Some(1_500),
                error_class: None,
                raw_error_message: None,
                retry_count: 0,
                completed_at: 101,
            }),
        ),
        EventEnvelope::for_runtime_event(
            EventId::new("evt_provider_call_run_2"),
            EventSource::Runtime,
            RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
                project,
                provider_call_id: ProviderCallId::new("pc_run_cost_2"),
                route_decision_id: RouteDecisionId::new("rd_run_cost_2"),
                route_attempt_id: RouteAttemptId::new("ra_run_cost_2"),
                provider_binding_id: ProviderBindingId::new("binding_cost"),
                provider_connection_id: ProviderConnectionId::new("conn_cost"),
                provider_model_id: ProviderModelId::new("model_cost"),
                run_id: Some(RunId::new("run_cost")),
                operation_kind: OperationKind::Generate,
                status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                latency_ms: Some(15),
                input_tokens: Some(50),
                output_tokens: Some(25),
                cost_micros: Some(2_500),
                error_class: None,
                raw_error_message: None,
                retry_count: 0,
                completed_at: 102,
            }),
        ),
    ];

    store.append(&events).await.unwrap();

    let run_cost = RunCostReadModel::get_run_cost(store.as_ref(), &RunId::new("run_cost"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run_cost.total_cost_micros, 4_000);
    assert_eq!(run_cost.provider_calls, 2);
    assert_eq!(run_cost.token_in, 150);
    assert_eq!(run_cost.token_out, 65);

    let streamed = store.read_stream(None, 20).await.unwrap();
    let run_cost_updates = streamed
        .iter()
        .filter(|stored| matches!(stored.envelope.payload, RuntimeEvent::RunCostUpdated(_)))
        .count();
    assert_eq!(run_cost_updates, 2);
}
