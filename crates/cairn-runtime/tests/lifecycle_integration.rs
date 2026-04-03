//! Integration tests for the core session -> run -> task lifecycle.
//! Uses InMemoryStore to test the full command -> event -> projection flow.

use std::sync::Arc;

use cairn_domain::*;
use cairn_runtime::{
    RunService, RunServiceImpl, SessionService, SessionServiceImpl, TaskService, TaskServiceImpl,
};
use cairn_store::InMemoryStore;

fn test_project() -> ProjectKey {
    ProjectKey::new("tenant_acme", "ws_main", "project_alpha")
}

#[tokio::test]
async fn create_session_persists_and_returns_open() {
    let store = Arc::new(InMemoryStore::new());
    let svc = SessionServiceImpl::new(store);
    let project = test_project();

    let session = svc
        .create(&project, SessionId::new("sess_1"))
        .await
        .unwrap();

    assert_eq!(session.state, SessionState::Open);
    assert_eq!(session.session_id, SessionId::new("sess_1"));
    assert_eq!(session.version, 1);
}

#[tokio::test]
async fn duplicate_session_returns_conflict() {
    let store = Arc::new(InMemoryStore::new());
    let svc = SessionServiceImpl::new(store);
    let project = test_project();

    svc.create(&project, SessionId::new("sess_1"))
        .await
        .unwrap();

    let result = svc.create(&project, SessionId::new("sess_1")).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn archive_session_transitions_to_archived() {
    let store = Arc::new(InMemoryStore::new());
    let svc = SessionServiceImpl::new(store);
    let project = test_project();

    svc.create(&project, SessionId::new("sess_1"))
        .await
        .unwrap();

    let archived = svc.archive(&SessionId::new("sess_1")).await.unwrap();
    assert_eq!(archived.state, SessionState::Archived);
}

#[tokio::test]
async fn run_lifecycle_pending_to_running_to_completed() {
    let store = Arc::new(InMemoryStore::new());
    let run_svc = RunServiceImpl::new(store.clone());
    let session_svc = SessionServiceImpl::new(store);
    let project = test_project();

    session_svc
        .create(&project, SessionId::new("sess_1"))
        .await
        .unwrap();

    // Start run
    let run = run_svc
        .start(
            &project,
            &SessionId::new("sess_1"),
            RunId::new("run_1"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Pending);

    // Move into active execution, then complete
    let run = run_svc
        .resume(
            &RunId::new("run_1"),
            ResumeTrigger::RuntimeSignal,
            RunResumeTarget::Running,
        )
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Running);

    let run = run_svc.complete(&RunId::new("run_1")).await.unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert!(run.state.is_terminal());
}

#[tokio::test]
async fn run_fail_with_failure_class() {
    let store = Arc::new(InMemoryStore::new());
    let run_svc = RunServiceImpl::new(store.clone());
    let session_svc = SessionServiceImpl::new(store);
    let project = test_project();

    session_svc
        .create(&project, SessionId::new("sess_1"))
        .await
        .unwrap();

    run_svc
        .start(
            &project,
            &SessionId::new("sess_1"),
            RunId::new("run_1"),
            None,
        )
        .await
        .unwrap();

    let run = run_svc
        .fail(&RunId::new("run_1"), FailureClass::TimedOut)
        .await
        .unwrap();

    assert_eq!(run.state, RunState::Failed);
    assert_eq!(run.failure_class, Some(FailureClass::TimedOut));
}

#[tokio::test]
async fn terminal_run_cannot_transition() {
    let store = Arc::new(InMemoryStore::new());
    let run_svc = RunServiceImpl::new(store.clone());
    let session_svc = SessionServiceImpl::new(store);
    let project = test_project();

    session_svc
        .create(&project, SessionId::new("sess_1"))
        .await
        .unwrap();
    run_svc
        .start(
            &project,
            &SessionId::new("sess_1"),
            RunId::new("run_1"),
            None,
        )
        .await
        .unwrap();
    run_svc
        .resume(
            &RunId::new("run_1"),
            ResumeTrigger::RuntimeSignal,
            RunResumeTarget::Running,
        )
        .await
        .unwrap();
    run_svc.complete(&RunId::new("run_1")).await.unwrap();

    let result = run_svc.cancel(&RunId::new("run_1")).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn task_submit_claim_start_complete_lifecycle() {
    let store = Arc::new(InMemoryStore::new());
    let task_svc = TaskServiceImpl::new(store);
    let project = test_project();

    // Submit
    let task = task_svc
        .submit(&project, TaskId::new("task_1"), None, None)
        .await
        .unwrap();
    assert_eq!(task.state, TaskState::Queued);

    // Claim
    let task = task_svc
        .claim(&TaskId::new("task_1"), "worker-a".to_owned(), 60_000)
        .await
        .unwrap();
    assert_eq!(task.state, TaskState::Leased);
    assert_eq!(task.lease_owner.as_deref(), Some("worker-a"));
    assert!(task.lease_expires_at.is_some());

    // Start
    let task = task_svc.start(&TaskId::new("task_1")).await.unwrap();
    assert_eq!(task.state, TaskState::Running);

    // Complete
    let task = task_svc.complete(&TaskId::new("task_1")).await.unwrap();
    assert_eq!(task.state, TaskState::Completed);
    assert!(task.state.is_terminal());
}

#[tokio::test]
async fn task_claim_requires_queued_state() {
    let store = Arc::new(InMemoryStore::new());
    let task_svc = TaskServiceImpl::new(store);
    let project = test_project();

    task_svc
        .submit(&project, TaskId::new("task_1"), None, None)
        .await
        .unwrap();
    task_svc
        .claim(&TaskId::new("task_1"), "worker-a".to_owned(), 60_000)
        .await
        .unwrap();

    // Can't claim again (already leased)
    let result = task_svc
        .claim(&TaskId::new("task_1"), "worker-b".to_owned(), 60_000)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn task_lease_expired_failure_is_retryable() {
    let store = Arc::new(InMemoryStore::new());
    let task_svc = TaskServiceImpl::new(store);
    let project = test_project();

    task_svc
        .submit(&project, TaskId::new("task_1"), None, None)
        .await
        .unwrap();
    task_svc
        .claim(&TaskId::new("task_1"), "worker-a".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(&TaskId::new("task_1")).await.unwrap();

    let task = task_svc
        .fail(&TaskId::new("task_1"), FailureClass::LeaseExpired)
        .await
        .unwrap();

    assert_eq!(task.state, TaskState::RetryableFailed);
    assert!(task.state.is_retryable());
    assert!(!task.state.is_terminal());
}

#[tokio::test]
async fn full_session_run_task_lifecycle() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let task_svc = TaskServiceImpl::new(store);
    let project = test_project();

    // Create session
    let session = session_svc
        .create(&project, SessionId::new("sess_1"))
        .await
        .unwrap();
    assert_eq!(session.state, SessionState::Open);

    // Start run
    let run = run_svc
        .start(
            &project,
            &SessionId::new("sess_1"),
            RunId::new("run_1"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Pending);
    let run = run_svc
        .resume(
            &RunId::new("run_1"),
            ResumeTrigger::RuntimeSignal,
            RunResumeTarget::Running,
        )
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Running);

    // Submit task linked to run
    let task = task_svc
        .submit(
            &project,
            TaskId::new("task_1"),
            Some(RunId::new("run_1")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(task.parent_run_id, Some(RunId::new("run_1")));

    // Work the task through its lifecycle
    task_svc
        .claim(&TaskId::new("task_1"), "worker-a".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(&TaskId::new("task_1")).await.unwrap();
    task_svc.complete(&TaskId::new("task_1")).await.unwrap();

    // Complete run
    run_svc.complete(&RunId::new("run_1")).await.unwrap();

    // Verify final states
    let session = session_svc
        .get(&SessionId::new("sess_1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.state, SessionState::Open); // still open (derivation happens at query time)

    let run = run_svc.get(&RunId::new("run_1")).await.unwrap().unwrap();
    assert_eq!(run.state, RunState::Completed);

    let task = task_svc.get(&TaskId::new("task_1")).await.unwrap().unwrap();
    assert_eq!(task.state, TaskState::Completed);
}
