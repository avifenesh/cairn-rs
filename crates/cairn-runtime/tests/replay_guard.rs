//! Replay regression guard: proves tool/external-worker events preserve
//! the same current-state reads after a full event stream rebuild.

use std::sync::Arc;

use cairn_domain::tool_invocation::ToolInvocationTarget;
use cairn_domain::workers::{ExternalWorkerOutcome, ExternalWorkerReport};
use cairn_domain::*;
use cairn_runtime::{
    ExternalWorkerService, ExternalWorkerServiceImpl, RunService, RunServiceImpl,
    RuntimeEnrichment, SessionService, SessionServiceImpl, StoreBackedEnrichment, TaskService,
    TaskServiceImpl, ToolInvocationService, ToolInvocationServiceImpl,
};
use cairn_store::projections::*;
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// Build a complete runtime state, capture current-state reads, replay
/// the full event stream into a fresh store, and verify reads match.
#[tokio::test]
async fn replay_preserves_current_state_reads() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let task_svc = TaskServiceImpl::new(store.clone());
    let tool_svc = ToolInvocationServiceImpl::new(store.clone());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());
    let p = project();

    // -- Build state --

    session_svc.create(&p, SessionId::new("s1")).await.unwrap();

    run_svc
        .start(&p, &SessionId::new("s1"), RunId::new("r1"), None)
        .await
        .unwrap();
    run_svc
        .resume(
            &RunId::new("r1"),
            ResumeTrigger::RuntimeSignal,
            RunResumeTarget::Running,
        )
        .await
        .unwrap();

    // Task with tool invocation
    task_svc
        .submit(&p, TaskId::new("t1"), Some(RunId::new("r1")), None)
        .await
        .unwrap();
    task_svc
        .claim(&TaskId::new("t1"), "w1".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(&TaskId::new("t1")).await.unwrap();

    tool_svc
        .record_start(
            &p,
            ToolInvocationId::new("inv1"),
            Some(SessionId::new("s1")),
            Some(RunId::new("r1")),
            Some(TaskId::new("t1")),
            ToolInvocationTarget::Builtin {
                tool_name: "fs.read".to_owned(),
            },
            ExecutionClass::SupervisedProcess,
        )
        .await
        .unwrap();
    tool_svc
        .record_completed(
            &p,
            ToolInvocationId::new("inv1"),
            Some(TaskId::new("t1")),
            "fs.read".to_owned(),
        )
        .await
        .unwrap();

    task_svc.complete(&TaskId::new("t1")).await.unwrap();

    // Task with external worker
    task_svc
        .submit(&p, TaskId::new("t2"), Some(RunId::new("r1")), None)
        .await
        .unwrap();
    task_svc
        .claim(&TaskId::new("t2"), "ext".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(&TaskId::new("t2")).await.unwrap();

    worker_svc
        .report(ExternalWorkerReport {
            project: p.clone(),
            worker_id: "ext".into(),
            run_id: None,
            task_id: TaskId::new("t2"),
            lease_token: 1,
            reported_at_ms: 12345,
            progress: None,
            outcome: Some(ExternalWorkerOutcome::Completed),
        })
        .await
        .unwrap();

    run_svc.complete(&RunId::new("r1")).await.unwrap();

    // -- Capture current-state reads from original store --

    let orig_session = SessionReadModel::get(store.as_ref(), &SessionId::new("s1"))
        .await
        .unwrap()
        .unwrap();
    let orig_run = RunReadModel::get(store.as_ref(), &RunId::new("r1"))
        .await
        .unwrap()
        .unwrap();
    let orig_task1 = TaskReadModel::get(store.as_ref(), &TaskId::new("t1"))
        .await
        .unwrap()
        .unwrap();
    let orig_task2 = TaskReadModel::get(store.as_ref(), &TaskId::new("t2"))
        .await
        .unwrap()
        .unwrap();

    // -- Replay into a fresh store --

    let all_events = store.read_stream(None, 10_000).await.unwrap();
    assert!(!all_events.is_empty(), "event stream should not be empty");

    let replay_store = Arc::new(InMemoryStore::new());
    let envelopes: Vec<_> = all_events.iter().map(|e| e.envelope.clone()).collect();
    replay_store.append(&envelopes).await.unwrap();

    // -- Verify replayed reads match original --

    let replay_session = SessionReadModel::get(replay_store.as_ref(), &SessionId::new("s1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replay_session.state, orig_session.state);
    assert_eq!(replay_session.version, orig_session.version);

    let replay_run = RunReadModel::get(replay_store.as_ref(), &RunId::new("r1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replay_run.state, orig_run.state);
    assert_eq!(replay_run.state, RunState::Completed);

    let replay_task1 = TaskReadModel::get(replay_store.as_ref(), &TaskId::new("t1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replay_task1.state, orig_task1.state);
    assert_eq!(replay_task1.state, TaskState::Completed);

    let replay_task2 = TaskReadModel::get(replay_store.as_ref(), &TaskId::new("t2"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replay_task2.state, orig_task2.state);
    assert_eq!(replay_task2.state, TaskState::Completed);

    // Event counts should match
    let replay_events = replay_store.read_stream(None, 10_000).await.unwrap();
    assert_eq!(replay_events.len(), all_events.len());

    // -- Enrichment works on replayed state --
    // This is the critical guard: if SSE/API enrichment breaks after
    // replay, Worker 8 surfaces stale or missing data.

    let enrichment = StoreBackedEnrichment::new(replay_store.clone());

    let task_e = enrichment
        .enrich_task(&TaskId::new("t1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task_e.state, TaskState::Completed);
    assert_eq!(task_e.lease_owner.as_deref(), Some("w1"));

    let session_e = enrichment
        .enrich_session(&SessionId::new("s1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session_e.state, SessionState::Completed);

    let run_e = enrichment
        .enrich_run(&RunId::new("r1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run_e.state, RunState::Completed);

    // External-worker-completed task also enrichable after replay
    let task2_e = enrichment
        .enrich_task(&TaskId::new("t2"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task2_e.state, TaskState::Completed);
}
