//! Integration tests for LLM observability (GAP-010).
use std::sync::Arc;

use cairn_domain::providers::{OperationKind, ProviderCallStatus};
use cairn_domain::{
    EventEnvelope, EventId, EventSource, LlmCallTrace, ProjectKey, ProviderBindingId,
    ProviderCallCompleted, ProviderCallId, ProviderConnectionId, ProviderModelId, RouteAttemptId,
    RouteDecisionId, RuntimeEvent, SessionId,
};
use cairn_runtime::{LlmObservabilityService, LlmObservabilityServiceImpl};
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t1", "w1", "p1")
}

fn make_call_completed(
    call_id: &str,
    model_id: &str,
    session_id: &str,
    input_tokens: u32,
    output_tokens: u32,
    latency_ms: u64,
    cost_micros: u64,
) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("evt_{call_id}")),
        EventSource::Runtime,
        RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
            project: project(),
            provider_call_id: ProviderCallId::new(call_id),
            route_decision_id: RouteDecisionId::new("rd_1"),
            route_attempt_id: RouteAttemptId::new("ra_1"),
            provider_binding_id: ProviderBindingId::new("pb_1"),
            provider_connection_id: ProviderConnectionId::new("pc_1"),
            provider_model_id: ProviderModelId::new(model_id),
            operation_kind: OperationKind::Generate,
            status: ProviderCallStatus::Succeeded,
            latency_ms: Some(latency_ms),
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            cost_micros: Some(cost_micros),
                error_class: None,
                raw_error_message: None,
                retry_count: 0,
                task_id: None,
                prompt_release_id: None,
                fallback_position: 0,
                started_at: 0,
                finished_at: 0,
            completed_at: 1_700_000_000_000 + latency_ms,
            session_id: Some(SessionId::new(session_id)),
            run_id: None,
        }),
    )
}

#[tokio::test]
async fn llm_observability_three_calls_recorded_for_session() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store.clone());
    let session_id = SessionId::new("sess_obs_1");

    // Append 3 ProviderCallCompleted events — projection auto-derives LlmCallTrace.
    store
        .append(&[
            make_call_completed("call_1", "claude-sonnet-4-6", "sess_obs_1", 100, 200, 500, 1000),
            make_call_completed("call_2", "claude-haiku-4-5", "sess_obs_1", 50, 100, 200, 400),
            make_call_completed("call_3", "claude-sonnet-4-6", "sess_obs_1", 150, 300, 600, 1500),
        ])
        .await
        .unwrap();

    let traces = svc.list_by_session(&session_id, 10).await.unwrap();

    assert_eq!(traces.len(), 3, "expected 3 traces for the session");

    // Verify model IDs are correct.
    let model_ids: Vec<&str> = traces.iter().map(|t| t.model_id.as_str()).collect();
    assert!(
        model_ids.contains(&"claude-sonnet-4-6"),
        "sonnet traces must be present"
    );
    assert!(
        model_ids.contains(&"claude-haiku-4-5"),
        "haiku trace must be present"
    );

    // Verify token counts are correct.
    let call1 = traces.iter().find(|t| t.trace_id == "call_1").unwrap();
    assert_eq!(call1.prompt_tokens, 100);
    assert_eq!(call1.completion_tokens, 200);
    assert_eq!(call1.latency_ms, 500);
    assert_eq!(call1.cost_micros, 1000);
    assert_eq!(call1.session_id, Some(session_id.clone()));
}

#[tokio::test]
async fn llm_observability_session_isolation() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store.clone());

    // Two calls for session A, one for session B.
    store
        .append(&[
            make_call_completed("c1", "model-a", "session_a", 10, 20, 100, 50),
            make_call_completed("c2", "model-a", "session_a", 10, 20, 100, 50),
            make_call_completed("c3", "model-b", "session_b", 30, 60, 200, 100),
        ])
        .await
        .unwrap();

    let traces_a = svc
        .list_by_session(&SessionId::new("session_a"), 10)
        .await
        .unwrap();
    let traces_b = svc
        .list_by_session(&SessionId::new("session_b"), 10)
        .await
        .unwrap();

    assert_eq!(traces_a.len(), 2, "session_a must have 2 traces");
    assert_eq!(traces_b.len(), 1, "session_b must have 1 trace");
    assert_eq!(traces_b[0].trace_id, "c3");
}

#[tokio::test]
async fn llm_observability_list_all_returns_all_traces() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store.clone());

    store
        .append(&[
            make_call_completed("c_all_1", "model-x", "s1", 10, 20, 100, 50),
            make_call_completed("c_all_2", "model-y", "s2", 10, 20, 100, 50),
            make_call_completed("c_all_3", "model-z", "s3", 10, 20, 100, 50),
        ])
        .await
        .unwrap();

    let all = svc.list_all(100).await.unwrap();
    assert!(all.len() >= 3);
    let ids: Vec<&str> = all.iter().map(|t| t.trace_id.as_str()).collect();
    assert!(ids.contains(&"c_all_1"));
    assert!(ids.contains(&"c_all_2"));
    assert!(ids.contains(&"c_all_3"));
}

#[tokio::test]
async fn llm_observability_direct_record() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store.clone());
    let session_id = SessionId::new("sess_direct");

    // Record directly without going through event log.
    svc.record(LlmCallTrace {
        trace_id: "direct_trace".to_owned(),
        model_id: "gpt-4o".to_owned(),
        prompt_tokens: 500,
        completion_tokens: 250,
        latency_ms: 1200,
        cost_micros: 5000,
        session_id: Some(session_id.clone()),
        run_id: None,
        created_at_ms: 1_700_000_000_000,
        is_error: false,
    })
    .await
    .unwrap();

    let traces = svc.list_by_session(&session_id, 5).await.unwrap();
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].model_id, "gpt-4o");
    assert_eq!(traces[0].prompt_tokens, 500);
}

#[tokio::test]
async fn llm_observability_empty_session_returns_empty() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store.clone());
    let traces = svc
        .list_by_session(&SessionId::new("no_such_session"), 10)
        .await
        .unwrap();
    assert!(traces.is_empty());
}
