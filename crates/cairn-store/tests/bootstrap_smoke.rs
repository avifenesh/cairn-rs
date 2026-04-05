//! Bootstrap smoke tests proving the event-sourcing pipeline works end-to-end.
//!
//! These tests verify that:
//! - InMemoryStore can be constructed and is empty on creation.
//! - Events can be appended and read back via EventLog.
//! - Appending a SessionCreated event creates the correct SessionRecord.
//! - Appending a RunCreated event creates the correct RunRecord.
//!
//! No cairn-app, cairn-runtime, or HTTP layer needed — pure store wiring.

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, TenantId, WorkspaceId, ProjectId,
};
use cairn_store::{
    projections::{RunReadModel, SessionReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn test_project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t1"),
        workspace_id: WorkspaceId::new("w1"),
        project_id: ProjectId::new("p1"),
    }
}

fn session_event(session_id: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("evt_{session_id}")),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            session_id: SessionId::new(session_id),
            project: test_project(),
        }),
    )
}

fn run_event(run_id: &str, session_id: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("evt_{run_id}")),
        EventSource::Runtime,
        RuntimeEvent::RunCreated(RunCreated {
            run_id: RunId::new(run_id),
            session_id: SessionId::new(session_id),
            project: test_project(),
            parent_run_id: None,
            prompt_release_id: None,
            agent_role_id: None,
        }),
    )
}

// ── 1. InMemoryStore construction ─────────────────────────────────────────────

#[tokio::test]
async fn store_constructs_and_is_empty() {
    let store = InMemoryStore::new();
    let head = store.head_position().await.unwrap();
    assert!(head.is_none(), "fresh store should have no events");
}

// ── 2. EventLog append + read_stream round-trip ───────────────────────────────

#[tokio::test]
async fn event_log_append_and_read_roundtrip() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[session_event("sess_rt")]).await.unwrap();

    let events = store.read_stream(None, 10).await.unwrap();
    assert_eq!(events.len(), 1, "should have exactly one event");

    match &events[0].envelope.payload {
        RuntimeEvent::SessionCreated(e) => {
            assert_eq!(e.session_id.as_str(), "sess_rt");
            assert_eq!(e.project.tenant_id.as_str(), "t1");
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let head = store.head_position().await.unwrap();
    assert!(head.is_some());
    assert_eq!(head.unwrap().0, 1);
}

#[tokio::test]
async fn event_log_read_after_position_skips_prior_events() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[session_event("s1")]).await.unwrap();
    let after_first = store.head_position().await.unwrap();

    store.append(&[session_event("s2")]).await.unwrap();

    let events = store.read_stream(after_first, 10).await.unwrap();
    assert_eq!(events.len(), 1, "should only return events after given position");
    match &events[0].envelope.payload {
        RuntimeEvent::SessionCreated(e) => assert_eq!(e.session_id.as_str(), "s2"),
        other => panic!("unexpected: {other:?}"),
    }
}

// ── 3. SyncProjection: SessionCreated produces SessionRecord ──────────────────

#[tokio::test]
async fn append_session_created_produces_session_record() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[session_event("sess_proj")]).await.unwrap();

    let record = SessionReadModel::get(store.as_ref(), &SessionId::new("sess_proj"))
        .await
        .unwrap();

    let record = record.expect("SessionRecord should exist after append");
    assert_eq!(record.session_id.as_str(), "sess_proj");
    assert_eq!(record.project, test_project());
}

// ── 4. SyncProjection: RunCreated produces RunRecord ─────────────────────────

#[tokio::test]
async fn append_run_created_produces_run_record() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[session_event("sess_run")]).await.unwrap();
    store.append(&[run_event("run_1", "sess_run")]).await.unwrap();

    let record = RunReadModel::get(store.as_ref(), &RunId::new("run_1"))
        .await
        .unwrap();

    let record = record.expect("RunRecord should exist after append");
    assert_eq!(record.run_id.as_str(), "run_1");
    assert_eq!(record.session_id.as_str(), "sess_run");
    assert_eq!(record.project, test_project());
}

// ── 5. Multiple events produce correct aggregate state ────────────────────────

#[tokio::test]
async fn multiple_sessions_and_runs_are_tracked_independently() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[session_event("sA"), session_event("sB")]).await.unwrap();
    store.append(&[run_event("rA1", "sA"), run_event("rA2", "sA")]).await.unwrap();
    store.append(&[run_event("rB1", "sB")]).await.unwrap();

    // All events are in the log (5 individual events across 3 append calls).
    let head = store.head_position().await.unwrap().unwrap();
    assert_eq!(head.0, 5, "5 events total");

    // Session records exist.
    assert!(SessionReadModel::get(store.as_ref(), &SessionId::new("sA")).await.unwrap().is_some());
    assert!(SessionReadModel::get(store.as_ref(), &SessionId::new("sB")).await.unwrap().is_some());

    // Run records exist and are correctly attributed to their sessions.
    let rA1 = RunReadModel::get(store.as_ref(), &RunId::new("rA1")).await.unwrap().unwrap();
    assert_eq!(rA1.session_id.as_str(), "sA");

    let rB1 = RunReadModel::get(store.as_ref(), &RunId::new("rB1")).await.unwrap().unwrap();
    assert_eq!(rB1.session_id.as_str(), "sB");

    // count_active_runs reflects all pending/running runs.
    let active = store.count_active_runs().await;
    assert_eq!(active, 3, "three pending runs");
}
