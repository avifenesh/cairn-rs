//! Smoke tests proving the control-plane bootstrap path is end-to-end wirable.
//!
//! These tests verify that:
//! - InMemoryStore and InMemoryServices can be constructed and wired.
//! - Core service traits are satisfied (compilation proves this).
//! - The event log append → read cycle works correctly.
//! - The AppBootstrap builds a working router.

#![cfg(feature = "in-memory-runtime")]

use std::sync::Arc;

use cairn_api::bootstrap::BootstrapConfig;
use cairn_app::AppBootstrap;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RuntimeEvent, SessionCreated,
    SessionId, TenantId, WorkspaceId,
};
use cairn_runtime::InMemoryServices;
use cairn_store::{EventLog, InMemoryStore};

// ── 1. InMemoryStore construction ─────────────────────────────────────────────

#[tokio::test]
async fn store_constructs_and_is_empty() {
    let store = InMemoryStore::new();
    let head = store.head_position().await.unwrap();
    assert!(head.is_none(), "fresh store should have no events");
}

// ── 2. InMemoryServices construction ─────────────────────────────────────────

#[test]
fn services_share_the_same_store() {
    let store = Arc::new(InMemoryStore::new());
    let _services = InMemoryServices::with_store(store.clone());
    // If this compiles and runs, all *ServiceImpl<InMemoryStore> trait bounds are satisfied.
}

// ── 3. Core service traits are wired ──────────────────────────────────────────

#[tokio::test]
async fn session_service_trait_is_implemented() {
    let services = InMemoryServices::new();
    let project = ProjectKey {
        tenant_id: TenantId::new("t1"),
        workspace_id: WorkspaceId::new("w1"),
        project_id: ProjectId::new("p1"),
    };

    // SessionService::create is the canonical entry-point for the runtime.
    // If this compiles, the trait impl for InMemoryStore is wired correctly.
    let result: Result<_, _> = services
        .sessions
        .create(&project, SessionId::new("sess_smoke_1"))
        .await;

    assert!(result.is_ok(), "create session should succeed: {result:?}");
    let session = result.unwrap();
    assert_eq!(session.session_id.as_str(), "sess_smoke_1");
}

// ── 4. Event log append → read cycle ─────────────────────────────────────────

#[tokio::test]
async fn event_log_append_and_read_roundtrip() {
    let store = Arc::new(InMemoryStore::new());

    let project = ProjectKey {
        tenant_id: TenantId::new("t1"),
        workspace_id: WorkspaceId::new("w1"),
        project_id: ProjectId::new("p1"),
    };

    let event = EventEnvelope::for_runtime_event(
        EventId::new("evt_smoke_1"),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            session_id: SessionId::new("sess_roundtrip"),
            project: project.clone(),
        }),
    );

    // Append
    let result = store.append(&[event]).await;
    assert!(result.is_ok(), "append should succeed: {result:?}");

    // Read back
    let events = store.read_stream(None, 10).await.unwrap();
    assert_eq!(events.len(), 1, "should have exactly one event");

    match &events[0].envelope.payload {
        RuntimeEvent::SessionCreated(e) => {
            assert_eq!(e.session_id.as_str(), "sess_roundtrip");
            assert_eq!(e.project.tenant_id.as_str(), "t1");
        }
        other => panic!("unexpected event type: {other:?}"),
    }

    // Head position reflects the appended event
    let head = store.head_position().await.unwrap();
    assert!(head.is_some(), "head should be Some after append");
    assert_eq!(head.unwrap().0, 1);
}

#[tokio::test]
async fn event_log_read_after_position_skips_prior_events() {
    let store = Arc::new(InMemoryStore::new());

    let project = ProjectKey {
        tenant_id: TenantId::new("t1"),
        workspace_id: WorkspaceId::new("w1"),
        project_id: ProjectId::new("p1"),
    };

    let mk_event = |id: &str, sess: &str| {
        EventEnvelope::for_runtime_event(
            EventId::new(id),
            EventSource::Runtime,
            RuntimeEvent::SessionCreated(SessionCreated {
                session_id: SessionId::new(sess),
                project: project.clone(),
            }),
        )
    };

    store.append(&[mk_event("e1", "s1")]).await.unwrap();
    let after_first = store.head_position().await.unwrap();

    store.append(&[mk_event("e2", "s2")]).await.unwrap();

    // Read after position 1 should only return the second event.
    let events = store.read_stream(after_first, 10).await.unwrap();
    assert_eq!(
        events.len(),
        1,
        "should only return events after the given position"
    );
    match &events[0].envelope.payload {
        RuntimeEvent::SessionCreated(e) => assert_eq!(e.session_id.as_str(), "s2"),
        other => panic!("unexpected: {other:?}"),
    }
}

// ── 5. AppBootstrap produces a working router ─────────────────────────────────

#[tokio::test]
async fn app_bootstrap_produces_valid_router() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let (app, _runtime, _tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();

    // The health endpoint should respond 200 with no auth required.
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "healthz should return 200"
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // /healthz reuses health_handler which returns {ok:true} or a status report
    assert!(json["ok"].as_bool() == Some(true) || json["status"].as_str().is_some());
}
