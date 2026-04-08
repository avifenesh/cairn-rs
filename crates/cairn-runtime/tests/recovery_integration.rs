//! Focused integration tests for recover_interrupted_runs and
//! resolve_stale_dependencies. These methods previously returned
//! silent zero-work summaries.

use std::sync::Arc;

use cairn_domain::*;
use cairn_runtime::{
    CheckpointService, CheckpointServiceImpl, RecoveryService, RecoveryServiceImpl, RunService,
    RunServiceImpl, SessionService, SessionServiceImpl, TaskService, TaskServiceImpl,
};
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// recover_interrupted_runs: finds a Running run without checkpoint and fails it.
#[tokio::test]
async fn recover_interrupted_run_without_checkpoint_fails_it() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let recovery_svc = RecoveryServiceImpl::new(store.clone());
    let p = project();

    // Create a session and a run, advance to Running
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

    // Run is now Running with no checkpoint — simulates interruption
    let summary = recovery_svc.recover_interrupted_runs(10).await.unwrap();

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.actions.len(), 1);
    assert!(matches!(
        summary.actions[0],
        cairn_runtime::RecoveryAction::RunFailed { .. }
    ));

    // Run should now be Failed
    let run = run_svc.get(&RunId::new("r1")).await.unwrap().unwrap();
    assert_eq!(run.state, RunState::Failed);

    // Recovery events should be in the stream
    let events = store.read_stream(None, 100).await.unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e.envelope.payload, RuntimeEvent::RecoveryAttempted(_))));
    assert!(events
        .iter()
        .any(|e| matches!(e.envelope.payload, RuntimeEvent::RecoveryCompleted(_))));
}

/// recover_interrupted_runs: finds a Running run WITH checkpoint and marks
/// it as resumed-from-checkpoint (doesn't fail it).
#[tokio::test]
async fn recover_interrupted_run_with_checkpoint_resumes() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let checkpoint_svc = CheckpointServiceImpl::new(store.clone());
    let recovery_svc = RecoveryServiceImpl::new(store.clone());
    let p = project();

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

    // Save a checkpoint before the "interruption"
    checkpoint_svc
        .save(&p, &RunId::new("r1"), CheckpointId::new("cp1"))
        .await
        .unwrap();

    let summary = recovery_svc.recover_interrupted_runs(10).await.unwrap();

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.actions.len(), 1);
    assert!(matches!(
        summary.actions[0],
        cairn_runtime::RecoveryAction::RunResumedFromCheckpoint { .. }
    ));

    // Run should still be Running (not failed) — checkpoint path doesn't
    // transition, it signals that resume is possible
    let run = run_svc.get(&RunId::new("r1")).await.unwrap().unwrap();
    assert_eq!(run.state, RunState::Running);
}

/// resolve_stale_dependencies: resumes WaitingDependency run when all children done.
#[tokio::test]
async fn resolve_stale_dependency_resumes_when_children_terminal() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let task_svc = TaskServiceImpl::new(store.clone());
    let recovery_svc = RecoveryServiceImpl::new(store.clone());
    let p = project();

    // Create parent session + run
    session_svc.create(&p, SessionId::new("s1")).await.unwrap();
    run_svc
        .start(&p, &SessionId::new("s1"), RunId::new("parent"), None)
        .await
        .unwrap();
    run_svc
        .resume(
            &RunId::new("parent"),
            ResumeTrigger::RuntimeSignal,
            RunResumeTarget::Running,
        )
        .await
        .unwrap();

    // Create child task linked to parent run
    task_svc
        .submit(
            &p,
            TaskId::new("child"),
            Some(RunId::new("parent")),
            None,
            0,
        )
        .await
        .unwrap();

    // Move parent to WaitingDependency
    store
        .append(&[cairn_domain::EventEnvelope {
            event_id: EventId::new("evt_wd"),
            source: EventSource::Runtime,
            ownership: OwnershipKey::Project(p.clone()),
            causation_id: None,
            correlation_id: None,
            payload: RuntimeEvent::RunStateChanged(RunStateChanged {
                project: p.clone(),
                run_id: RunId::new("parent"),
                transition: StateTransition {
                    from: Some(RunState::Running),
                    to: RunState::WaitingDependency,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
        }])
        .await
        .unwrap();

    // Child is still Queued (non-terminal) — should NOT resume parent
    let summary = recovery_svc.resolve_stale_dependencies(10).await.unwrap();
    assert_eq!(summary.scanned, 1);
    assert_eq!(
        summary.actions.len(),
        0,
        "should not resume: child still active"
    );

    let run = run_svc.get(&RunId::new("parent")).await.unwrap().unwrap();
    assert_eq!(run.state, RunState::WaitingDependency);

    // Complete the child task
    task_svc
        .claim(&TaskId::new("child"), "w".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(&TaskId::new("child")).await.unwrap();
    task_svc.complete(&TaskId::new("child")).await.unwrap();

    // Now recovery should resume parent
    let summary = recovery_svc.resolve_stale_dependencies(10).await.unwrap();
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.actions.len(), 1);
    assert!(matches!(
        summary.actions[0],
        cairn_runtime::RecoveryAction::DependencyResolved { .. }
    ));

    let run = run_svc.get(&RunId::new("parent")).await.unwrap().unwrap();
    assert_eq!(run.state, RunState::Running);
}

/// No interrupted runs → zero-work summary (not a silent placeholder).
#[tokio::test]
async fn no_interrupted_runs_returns_empty_summary() {
    let store = Arc::new(InMemoryStore::new());
    let recovery_svc = RecoveryServiceImpl::new(store);

    let summary = recovery_svc.recover_interrupted_runs(10).await.unwrap();
    assert_eq!(summary.scanned, 0);
    assert!(summary.actions.is_empty());
}
