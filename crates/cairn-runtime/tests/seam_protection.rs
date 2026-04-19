#![cfg(feature = "in-memory-runtime")]

//! Integration coverage protecting the tool invocation and external worker
//! seams for Workers 5 and 8. Manager directive: narrow coverage against drift.

use std::sync::Arc;

use cairn_domain::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget};
use cairn_domain::workers::{ExternalWorkerOutcome, ExternalWorkerReport};
use cairn_domain::*;
use cairn_runtime::{
    ExternalWorkerService, ExternalWorkerServiceImpl, TaskService, TaskServiceImpl,
    ToolInvocationService, ToolInvocationServiceImpl,
};
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

// -- ToolInvocationService seam --

#[tokio::test]
async fn tool_invocation_start_emits_event() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ToolInvocationServiceImpl::new(store.clone());

    svc.record_start(
        &project(),
        ToolInvocationId::new("inv_1"),
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

    let events = store.read_stream(None, 10).await.unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].envelope.payload,
        RuntimeEvent::ToolInvocationStarted(_)
    ));
}

#[tokio::test]
async fn tool_invocation_complete_emits_event() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ToolInvocationServiceImpl::new(store.clone());

    svc.record_start(
        &project(),
        ToolInvocationId::new("inv_1"),
        None,
        Some(RunId::new("r1")),
        None,
        ToolInvocationTarget::Plugin {
            plugin_id: "com.example".to_owned(),
            tool_name: "test.tool".to_owned(),
        },
        ExecutionClass::SandboxedProcess,
    )
    .await
    .unwrap();

    svc.record_completed(
        &project(),
        ToolInvocationId::new("inv_1"),
        None,
        "test.tool".to_owned(),
    )
    .await
    .unwrap();

    let events = store.read_stream(None, 10).await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[1].envelope.payload,
        RuntimeEvent::ToolInvocationCompleted(_)
    ));
}

#[tokio::test]
async fn tool_invocation_failed_emits_event_with_outcome() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ToolInvocationServiceImpl::new(store.clone());

    svc.record_start(
        &project(),
        ToolInvocationId::new("inv_1"),
        None,
        Some(RunId::new("r1")),
        None,
        ToolInvocationTarget::Builtin {
            tool_name: "fs.write".to_owned(),
        },
        ExecutionClass::SupervisedProcess,
    )
    .await
    .unwrap();

    svc.record_failed(
        &project(),
        ToolInvocationId::new("inv_1"),
        None,
        "fs.write".to_owned(),
        ToolInvocationOutcomeKind::PermanentFailure,
        Some("permission denied".to_owned()),
    )
    .await
    .unwrap();

    let events = store.read_stream(None, 10).await.unwrap();
    assert_eq!(events.len(), 2);
    match &events[1].envelope.payload {
        RuntimeEvent::ToolInvocationFailed(f) => {
            assert_eq!(f.tool_name, "fs.write");
            assert_eq!(f.outcome, ToolInvocationOutcomeKind::PermanentFailure);
            assert_eq!(f.error_message.as_deref(), Some("permission denied"));
        }
        other => panic!("expected ToolInvocationFailed, got {other:?}"),
    }
}

// -- ExternalWorkerService seam --

#[tokio::test]
async fn external_worker_progress_report_emits_event() {
    let store = Arc::new(InMemoryStore::new());
    let task_svc = TaskServiceImpl::new(store.clone());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());

    task_svc
        .submit(&project(), None, TaskId::new("task_1"), None, None, 0)
        .await
        .unwrap();
    task_svc
        .claim(None, &TaskId::new("task_1"), "worker-ext".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(None, &TaskId::new("task_1")).await.unwrap();

    // Progress report (no outcome)
    let report = ExternalWorkerReport {
        project: project(),
        worker_id: "worker-ext".into(),
        run_id: None,
        task_id: TaskId::new("task_1"),
        lease_token: 1,
        reported_at_ms: 12345,
        progress: Some(cairn_domain::workers::ExternalWorkerProgress {
            message: Some("50% done".to_owned()),
            percent_milli: Some(500),
        }),
        outcome: None,
    };

    worker_svc.report(report).await.unwrap();

    let events = store.read_stream(None, 100).await.unwrap();
    let worker_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.envelope.payload, RuntimeEvent::ExternalWorkerReported(_)))
        .collect();
    assert_eq!(worker_events.len(), 1);
}

#[tokio::test]
async fn external_worker_completed_transitions_task() {
    let store = Arc::new(InMemoryStore::new());
    let task_svc = TaskServiceImpl::new(store.clone());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());

    task_svc
        .submit(&project(), None, TaskId::new("task_1"), None, None, 0)
        .await
        .unwrap();
    task_svc
        .claim(None, &TaskId::new("task_1"), "worker-ext".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(None, &TaskId::new("task_1")).await.unwrap();

    // Terminal report
    let report = ExternalWorkerReport {
        project: project(),
        worker_id: "worker-ext".into(),
        run_id: None,
        task_id: TaskId::new("task_1"),
        lease_token: 1,
        reported_at_ms: 99999,
        progress: None,
        outcome: Some(ExternalWorkerOutcome::Completed),
    };

    worker_svc.report(report).await.unwrap();

    let task = task_svc.get(&TaskId::new("task_1")).await.unwrap().unwrap();
    assert_eq!(task.state, TaskState::Completed);
}

#[tokio::test]
async fn external_worker_report_on_terminal_task_fails() {
    let store = Arc::new(InMemoryStore::new());
    let task_svc = TaskServiceImpl::new(store.clone());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());

    task_svc
        .submit(&project(), None, TaskId::new("task_1"), None, None, 0)
        .await
        .unwrap();
    task_svc
        .claim(None, &TaskId::new("task_1"), "worker-ext".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(None, &TaskId::new("task_1")).await.unwrap();
    task_svc.complete(None, &TaskId::new("task_1")).await.unwrap();

    // Report on already-completed task
    let report = ExternalWorkerReport {
        project: project(),
        worker_id: "worker-ext".into(),
        run_id: None,
        task_id: TaskId::new("task_1"),
        lease_token: 1,
        reported_at_ms: 99999,
        progress: None,
        outcome: None,
    };

    let result = worker_svc.report(report).await;
    assert!(result.is_err());
}
