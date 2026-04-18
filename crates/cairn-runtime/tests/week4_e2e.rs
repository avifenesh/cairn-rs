//! Week 4: End-to-end runtime slice from command through replay.
//!
//! Recovery coverage that lived here was removed in the Fabric finalization
//! round — FF's LeaseExpiryScanner, AttemptTimeoutScanner, and
//! DependencyReconciler own the recovery paths that the deleted test
//! `recovery_produces_auditable_events` used to exercise.
//! Proves the full command → event → projection → replay cycle works.

use std::sync::Arc;

use cairn_domain::*;
use cairn_runtime::{
    ApprovalService, ApprovalServiceImpl, CheckpointService, CheckpointServiceImpl, MailboxService,
    MailboxServiceImpl, RunService, RunServiceImpl, SessionService, SessionServiceImpl,
    TaskService, TaskServiceImpl,
};
use cairn_store::{EventLog, InMemoryStore};

fn test_project() -> ProjectKey {
    ProjectKey::new("tenant_acme", "ws_main", "project_alpha")
}

/// Full runtime slice: session → run → task → approval → checkpoint → mailbox → complete.
/// Then replay the event stream and verify all events are present and ordered.
#[tokio::test]
async fn end_to_end_runtime_slice_with_replay() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let task_svc = TaskServiceImpl::new(store.clone());
    let approval_svc = ApprovalServiceImpl::new(store.clone());
    let checkpoint_svc = CheckpointServiceImpl::new(store.clone());
    let mailbox_svc = MailboxServiceImpl::new(store.clone());

    let project = test_project();

    // 1. Create session
    session_svc
        .create(&project, SessionId::new("sess_e2e"))
        .await
        .unwrap();

    // 2. Start run
    run_svc
        .start(
            &project,
            &SessionId::new("sess_e2e"),
            RunId::new("run_e2e"),
            None,
        )
        .await
        .unwrap();

    // 3. Advance run to Running
    run_svc
        .resume(
            &RunId::new("run_e2e"),
            ResumeTrigger::RuntimeSignal,
            RunResumeTarget::Running,
        )
        .await
        .unwrap();

    // 4. Submit and work a task
    task_svc
        .submit(
            &project,
            TaskId::new("task_e2e"),
            Some(RunId::new("run_e2e")),
            None,
            0,
        )
        .await
        .unwrap();
    task_svc
        .claim(&TaskId::new("task_e2e"), "worker-a".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(&TaskId::new("task_e2e")).await.unwrap();

    // 5. Request and resolve approval
    approval_svc
        .request(
            &project,
            ApprovalId::new("appr_e2e"),
            Some(RunId::new("run_e2e")),
            Some(TaskId::new("task_e2e")),
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();
    approval_svc
        .resolve(&ApprovalId::new("appr_e2e"), ApprovalDecision::Approved)
        .await
        .unwrap();

    // 6. Save checkpoint
    checkpoint_svc
        .save(
            &project,
            &RunId::new("run_e2e"),
            CheckpointId::new("cp_e2e"),
        )
        .await
        .unwrap();

    // 7. Send mailbox message
    mailbox_svc
        .append(
            &project,
            MailboxMessageId::new("msg_e2e"),
            Some(RunId::new("run_e2e")),
            Some(TaskId::new("task_e2e")),
            "".to_owned(),
            None,
            0,
        )
        .await
        .unwrap();

    // 8. Complete task and run
    task_svc.complete(&TaskId::new("task_e2e")).await.unwrap();
    run_svc.complete(&RunId::new("run_e2e")).await.unwrap();

    // 9. Archive session
    session_svc
        .archive(&SessionId::new("sess_e2e"))
        .await
        .unwrap();

    // -- REPLAY: Read back the entire event stream --
    let all_events = store.read_stream(None, 1000).await.unwrap();

    // Verify we have a reasonable number of events covering all entity types
    assert!(
        all_events.len() >= 12,
        "expected at least 12 events, got {}",
        all_events.len()
    );

    // Verify events are monotonically ordered
    for window in all_events.windows(2) {
        assert!(
            window[0].position < window[1].position,
            "events not monotonically ordered: {:?} >= {:?}",
            window[0].position,
            window[1].position
        );
    }

    // Verify specific event types are present
    let has_session_created = all_events
        .iter()
        .any(|e| matches!(e.envelope.payload, RuntimeEvent::SessionCreated(_)));
    let has_run_created = all_events
        .iter()
        .any(|e| matches!(e.envelope.payload, RuntimeEvent::RunCreated(_)));
    let has_task_created = all_events
        .iter()
        .any(|e| matches!(e.envelope.payload, RuntimeEvent::TaskCreated(_)));
    let has_approval = all_events
        .iter()
        .any(|e| matches!(e.envelope.payload, RuntimeEvent::ApprovalRequested(_)));
    let has_checkpoint = all_events
        .iter()
        .any(|e| matches!(e.envelope.payload, RuntimeEvent::CheckpointRecorded(_)));
    let has_mailbox = all_events
        .iter()
        .any(|e| matches!(e.envelope.payload, RuntimeEvent::MailboxMessageAppended(_)));

    assert!(has_session_created, "missing SessionCreated event");
    assert!(has_run_created, "missing RunCreated event");
    assert!(has_task_created, "missing TaskCreated event");
    assert!(has_approval, "missing ApprovalRequested event");
    assert!(has_checkpoint, "missing CheckpointRecorded event");
    assert!(has_mailbox, "missing MailboxMessageAppended event");

    // Verify final entity states from projections
    let session = session_svc
        .get(&SessionId::new("sess_e2e"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.state, SessionState::Archived);

    let run = run_svc.get(&RunId::new("run_e2e")).await.unwrap().unwrap();
    assert_eq!(run.state, RunState::Completed);

    let task = task_svc
        .get(&TaskId::new("task_e2e"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.state, TaskState::Completed);

    let approval = approval_svc
        .get(&ApprovalId::new("appr_e2e"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approval.decision, Some(ApprovalDecision::Approved));

    let checkpoint = checkpoint_svc
        .get(&CheckpointId::new("cp_e2e"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.disposition, CheckpointDisposition::Latest);
}

/// Subagent spawn: parent run creates child task + session, child completes.
#[tokio::test]
async fn subagent_spawn_creates_linked_entities() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let task_svc = TaskServiceImpl::new(store.clone());

    let project = test_project();

    // Parent session and run
    session_svc
        .create(&project, SessionId::new("parent_sess"))
        .await
        .unwrap();
    run_svc
        .start(
            &project,
            &SessionId::new("parent_sess"),
            RunId::new("parent_run"),
            None,
        )
        .await
        .unwrap();

    // Subagent spawn: create child task linked to parent run
    let child_task = task_svc
        .submit(
            &project,
            TaskId::new("child_task"),
            Some(RunId::new("parent_run")),
            None,
            0,
        )
        .await
        .unwrap();
    assert_eq!(child_task.parent_run_id, Some(RunId::new("parent_run")));

    // Create child session for the subagent
    session_svc
        .create(&project, SessionId::new("child_sess"))
        .await
        .unwrap();

    // Child run inside child session
    let child_run = run_svc
        .start(
            &project,
            &SessionId::new("child_sess"),
            RunId::new("child_run"),
            Some(RunId::new("parent_run")),
        )
        .await
        .unwrap();
    assert_eq!(child_run.parent_run_id, Some(RunId::new("parent_run")));

    // Work and complete child
    task_svc
        .claim(&TaskId::new("child_task"), "worker-b".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(&TaskId::new("child_task")).await.unwrap();
    task_svc.complete(&TaskId::new("child_task")).await.unwrap();

    run_svc
        .resume(
            &RunId::new("child_run"),
            ResumeTrigger::RuntimeSignal,
            RunResumeTarget::Running,
        )
        .await
        .unwrap();
    run_svc.complete(&RunId::new("child_run")).await.unwrap();

    // Complete parent
    run_svc
        .resume(
            &RunId::new("parent_run"),
            ResumeTrigger::RuntimeSignal,
            RunResumeTarget::Running,
        )
        .await
        .unwrap();
    run_svc.complete(&RunId::new("parent_run")).await.unwrap();

    // Verify linkage and final states
    let child_task = task_svc
        .get(&TaskId::new("child_task"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child_task.state, TaskState::Completed);
    assert_eq!(child_task.parent_run_id, Some(RunId::new("parent_run")));

    let child_run = run_svc
        .get(&RunId::new("child_run"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child_run.state, RunState::Completed);
    assert_eq!(child_run.parent_run_id, Some(RunId::new("parent_run")));

    let parent_run = run_svc
        .get(&RunId::new("parent_run"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parent_run.state, RunState::Completed);
}

// `recovery_produces_auditable_events` deleted in the Fabric finalization
// round. FF's LeaseExpiryScanner + AttemptTimeoutScanner emit equivalent
// events into exec_core / lease_history / the event bridge — if cairn
// wants auditable proof of a recovery sweep under FabricEnabled=1, read
// the EventBridge sink directly. Under the in-memory dev path the sweep
// has nothing to recover (no background scanner running) so the test
// would have no useful invariant to check.
