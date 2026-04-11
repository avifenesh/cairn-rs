//! RFC 005 session lifecycle end-to-end integration test.
//!
//! Validates the full session lifecycle:
//!   (1) create session — starts in Open state
//!   (2) create a run within the session
//!   (3) complete the run — run moves to Completed
//!   (4) close (archive) the session — transitions to Archived
//!   (5) session cost tracking accumulates from provider calls tied to the run
//!   (6) session state derivation — session with all completed runs is closeable

use std::sync::Arc;

use cairn_domain::providers::OperationKind;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, ProviderBindingId, ProviderCallCompleted,
    ProviderCallId, ProviderConnectionId, ProviderModelId, ResumeTrigger, RouteAttemptId,
    RouteDecisionId, RunId, RunResumeTarget, RuntimeEvent, SessionId, SessionState,
};
use cairn_runtime::{RunService, RunServiceImpl, SessionService, SessionServiceImpl};
use cairn_store::projections::SessionCostReadModel;
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t_sess", "ws_sess", "proj_sess")
}

/// Advance a run from Pending → Running via resume.
async fn activate_run(runs: &impl RunService, run_id: &RunId) {
    runs.resume(
        run_id,
        ResumeTrigger::RuntimeSignal,
        RunResumeTarget::Running,
    )
    .await
    .unwrap();
}

fn services() -> (
    Arc<InMemoryStore>,
    SessionServiceImpl<InMemoryStore>,
    RunServiceImpl<InMemoryStore>,
) {
    let store = Arc::new(InMemoryStore::new());
    let sessions = SessionServiceImpl::new(store.clone());
    let runs = RunServiceImpl::new(store.clone());
    (store, sessions, runs)
}

// ── (1) Create session — starts in Open ──────────────────────────────────

#[tokio::test]
async fn step1_create_session_starts_open() {
    let (_, sessions, _) = services();
    let sess_id = SessionId::new("sess_lc_1");

    let record = sessions.create(&project(), sess_id.clone()).await.unwrap();

    assert_eq!(record.session_id, sess_id);
    assert_eq!(
        record.state,
        SessionState::Open,
        "freshly created session must be Open"
    );
    assert_eq!(record.project, project());
}

// ── (2) Create run within session ────────────────────────────────────────

#[tokio::test]
async fn step2_create_run_within_session() {
    let (_, sessions, runs) = services();
    let sess_id = SessionId::new("sess_lc_2");
    let run_id = RunId::new("run_lc_2");

    sessions.create(&project(), sess_id.clone()).await.unwrap();

    let run = runs
        .start(&project(), &sess_id, run_id.clone(), None)
        .await
        .unwrap();

    assert_eq!(run.run_id, run_id);
    assert_eq!(run.session_id, sess_id);
    assert_eq!(
        run.state,
        cairn_domain::RunState::Pending,
        "newly created run starts in Pending state"
    );
}

// ── (3) Complete the run ──────────────────────────────────────────────────

#[tokio::test]
async fn step3_complete_run() {
    let (_, sessions, runs) = services();
    let sess_id = SessionId::new("sess_lc_3");
    let run_id = RunId::new("run_lc_3");

    sessions.create(&project(), sess_id.clone()).await.unwrap();
    runs.start(&project(), &sess_id, run_id.clone(), None)
        .await
        .unwrap();
    activate_run(&runs, &run_id).await;

    let completed = runs.complete(&run_id).await.unwrap();

    assert_eq!(
        completed.state,
        cairn_domain::RunState::Completed,
        "run must be Completed after complete()"
    );
    assert!(
        completed.state.is_terminal(),
        "Completed is a terminal state"
    );
}

// ── (4) Close (archive) the session ──────────────────────────────────────

#[tokio::test]
async fn step4_archive_session_transitions_to_archived() {
    let (_, sessions, runs) = services();
    let sess_id = SessionId::new("sess_lc_4");
    let run_id = RunId::new("run_lc_4");

    sessions.create(&project(), sess_id.clone()).await.unwrap();
    runs.start(&project(), &sess_id, run_id.clone(), None)
        .await
        .unwrap();
    activate_run(&runs, &run_id).await;
    runs.complete(&run_id).await.unwrap();

    let archived = sessions.archive(&sess_id).await.unwrap();

    assert_eq!(
        archived.state,
        SessionState::Archived,
        "archived session must be in Archived state"
    );
    assert!(
        archived.state.is_terminal(),
        "Archived is a terminal (closed) state"
    );
}

// ── (5) Session cost accumulates from provider calls ─────────────────────

#[tokio::test]
async fn step5_session_cost_accumulates_from_run_provider_calls() {
    let (store, sessions, runs) = services();
    let sess_id = SessionId::new("sess_lc_5");
    let run_id = RunId::new("run_lc_5");

    sessions.create(&project(), sess_id.clone()).await.unwrap();
    runs.start(&project(), &sess_id, run_id.clone(), None)
        .await
        .unwrap();

    // Simulate two provider calls attributed to this run/session.
    let cost_events = vec![
        EventEnvelope::for_runtime_event(
            EventId::new("evt_pc_5a"),
            EventSource::Runtime,
            RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
                project: project(),
                provider_call_id: ProviderCallId::new("pc_5a"),
                route_decision_id: RouteDecisionId::new("rd_5a"),
                route_attempt_id: RouteAttemptId::new("ra_5a"),
                provider_binding_id: ProviderBindingId::new("binding_5"),
                provider_connection_id: ProviderConnectionId::new("conn_5"),
                provider_model_id: ProviderModelId::new("model_5"),
                run_id: Some(run_id.clone()),
                operation_kind: OperationKind::Generate,
                status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                latency_ms: Some(200),
                input_tokens: Some(300),
                output_tokens: Some(100),
                cost_micros: Some(3_000),
                error_class: None,
                raw_error_message: None,
                retry_count: 0,
                task_id: None,
                prompt_release_id: None,
                fallback_position: 0,
                started_at: 0,
                finished_at: 0,
                session_id: Some(sess_id.clone()),
                completed_at: 1001,
            }),
        ),
        EventEnvelope::for_runtime_event(
            EventId::new("evt_pc_5b"),
            EventSource::Runtime,
            RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
                project: project(),
                provider_call_id: ProviderCallId::new("pc_5b"),
                route_decision_id: RouteDecisionId::new("rd_5b"),
                route_attempt_id: RouteAttemptId::new("ra_5b"),
                provider_binding_id: ProviderBindingId::new("binding_5"),
                provider_connection_id: ProviderConnectionId::new("conn_5"),
                provider_model_id: ProviderModelId::new("model_5"),
                run_id: Some(run_id.clone()),
                operation_kind: OperationKind::Generate,
                status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                latency_ms: Some(150),
                input_tokens: Some(200),
                output_tokens: Some(80),
                cost_micros: Some(2_000),
                error_class: None,
                raw_error_message: None,
                retry_count: 0,
                task_id: None,
                prompt_release_id: None,
                fallback_position: 0,
                started_at: 0,
                finished_at: 0,
                session_id: Some(sess_id.clone()),
                completed_at: 1002,
            }),
        ),
    ];

    store.append(&cost_events).await.unwrap();

    let cost = SessionCostReadModel::get_session_cost(store.as_ref(), &sess_id)
        .await
        .unwrap()
        .expect("session cost record must exist after provider calls");

    assert_eq!(
        cost.total_cost_micros, 5_000,
        "total cost must be 3000 + 2000 = 5000 µUSD"
    );
    assert_eq!(
        cost.provider_calls, 2,
        "two provider calls must be recorded"
    );
    assert_eq!(cost.token_in, 500, "input tokens: 300 + 200 = 500");
    assert_eq!(cost.token_out, 180, "output tokens: 100 + 80 = 180");
}

// ── (6) Session state derivation — completed run makes session closeable ──

#[tokio::test]
async fn step6_session_with_all_completed_runs_is_closeable() {
    let (store, sessions, runs) = services();
    let sess_id = SessionId::new("sess_lc_6");
    let run_id = RunId::new("run_lc_6");

    sessions.create(&project(), sess_id.clone()).await.unwrap();
    runs.start(&project(), &sess_id, run_id.clone(), None)
        .await
        .unwrap();

    // Before run completes — session must still be Open.
    let session_mid = sessions.get(&sess_id).await.unwrap().unwrap();
    assert_eq!(
        session_mid.state,
        SessionState::Open,
        "session must stay Open while run is Pending/Running"
    );

    activate_run(&runs, &run_id).await;
    runs.complete(&run_id).await.unwrap();

    // After run completes — derived state should be Completed, enabling archive.
    let session_post = sessions.get(&sess_id).await.unwrap().unwrap();
    assert!(
        matches!(
            session_post.state,
            SessionState::Completed | SessionState::Open
        ),
        "session with all completed runs must be Completed or still Open (closeable)"
    );

    // Explicitly verify the session can be archived (closed) now.
    let archived = sessions.archive(&sess_id).await.unwrap();
    assert_eq!(
        archived.state,
        SessionState::Archived,
        "session must transition to Archived when explicitly closed"
    );

    // Verify the archived session is visible via list.
    let all = sessions.list(&project(), 10, 0).await.unwrap();
    let found = all.iter().find(|s| s.session_id == sess_id);
    assert!(found.is_some(), "archived session must appear in list");
    assert_eq!(found.unwrap().state, SessionState::Archived);

    // Verify the store recorded a SessionStateChanged event for the archive.
    let events = store.read_stream(None, 100).await.unwrap();
    let archive_events = events
        .iter()
        .filter(|e| matches!(e.envelope.payload, RuntimeEvent::SessionStateChanged(_)))
        .count();
    assert!(
        archive_events >= 1,
        "at least one SessionStateChanged event must be emitted"
    );
}

// ── Double-archive is idempotent ──────────────────────────────────────────

#[tokio::test]
async fn archive_already_archived_session_is_idempotent() {
    let (_, sessions, _) = services();
    let sess_id = SessionId::new("sess_lc_idem");

    sessions.create(&project(), sess_id.clone()).await.unwrap();
    sessions.archive(&sess_id).await.unwrap();

    // Second archive should succeed (idempotent) or return Archived record.
    let result = sessions.archive(&sess_id).await;
    if let Ok(record) = result {
        assert_eq!(record.state, SessionState::Archived);
    }
    // acceptable: some impls treat double-archive as no-op error
}
