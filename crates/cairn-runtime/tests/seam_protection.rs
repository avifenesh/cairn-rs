//! Integration coverage protecting the tool-invocation seam and the
//! external-worker progress-report seam.
//!
//! The services under test (`ToolInvocationService`, `ExternalWorkerService`)
//! are projection-backed and independent of Run/Task/Session impls —
//! they append events to the store on each call. These tests verify the
//! seam emits the right events.
//!
//! Task-state coordination for external workers (terminal-report rejection,
//! worker-report-transitions-task) lives in
//! `crates/cairn-fabric/tests/integration/test_external_worker.rs` against
//! live Valkey — that's where the real coordination happens in production.

use std::sync::Arc;

use cairn_domain::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget};
use cairn_domain::workers::ExternalWorkerReport;
use cairn_domain::*;
use cairn_runtime::{
    ExternalWorkerService, ExternalWorkerServiceImpl, ToolInvocationService,
    ToolInvocationServiceImpl,
};
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// Seed a task directly via envelope append. The seam services read only
/// current-state projections, not service state, so this is enough to
/// drive worker reports.
async fn seed_running_task(store: &Arc<InMemoryStore>, task_id: &str) {
    let created = EventEnvelope::for_runtime_event(
        EventId::new(format!("task_created_{task_id}")),
        EventSource::Runtime,
        RuntimeEvent::TaskCreated(TaskCreated {
            project: project(),
            task_id: TaskId::new(task_id),
            parent_run_id: None,
            parent_task_id: None,
            prompt_release_id: None,
            session_id: None,
        }),
    );
    let to_leased = EventEnvelope::for_runtime_event(
        EventId::new(format!("task_leased_{task_id}")),
        EventSource::Runtime,
        RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: project(),
            task_id: TaskId::new(task_id),
            transition: StateTransition {
                from: Some(TaskState::Queued),
                to: TaskState::Leased,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }),
    );
    let to_running = EventEnvelope::for_runtime_event(
        EventId::new(format!("task_running_{task_id}")),
        EventSource::Runtime,
        RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: project(),
            task_id: TaskId::new(task_id),
            transition: StateTransition {
                from: Some(TaskState::Leased),
                to: TaskState::Running,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }),
    );
    store
        .append(&[created, to_leased, to_running])
        .await
        .unwrap();
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
        &[], None, None,
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

// -- ExternalWorkerService seam (progress emission only) --
//
// Terminal-report rejection and worker-completed-transitions-task moved to
// Fabric integration (`test_external_worker.rs`) in commit 3 — those
// assertions depend on real task-state coordination which FF owns in
// production.

#[tokio::test]
async fn external_worker_progress_report_emits_event() {
    let store = Arc::new(InMemoryStore::new());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());

    seed_running_task(&store, "task_1").await;

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
