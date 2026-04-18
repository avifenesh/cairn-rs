#![cfg(feature = "in-memory-runtime")]

//! RFC 002 snapshot create-and-restore integration test.

use std::sync::Arc;

use cairn_domain::*;
use cairn_runtime::services::{
    ProjectServiceImpl, RunServiceImpl, SessionServiceImpl, TaskServiceImpl, TenantServiceImpl,
    WorkspaceServiceImpl,
};
use cairn_runtime::{ProjectService, SessionService, TaskService, TenantService, WorkspaceService};
use cairn_store::projections::RunReadModel;
use cairn_store::{EventLog, InMemoryStore};

#[tokio::test]
async fn snapshot_create_restore_preserves_run_state() {
    let store = Arc::new(InMemoryStore::new());

    let tenant_svc = TenantServiceImpl::new(store.clone());
    let workspace_svc = WorkspaceServiceImpl::new(store.clone());
    let project_svc = ProjectServiceImpl::new(store.clone());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let task_svc = TaskServiceImpl::new(store.clone());

    let tenant_id = TenantId::new("tenant_snap");
    let workspace_id = WorkspaceId::new("ws_snap");
    let project = ProjectKey::new("tenant_snap", "ws_snap", "proj_snap");
    let session_id = SessionId::new("sess_snap");
    let run_id = RunId::new("run_snap");

    // Build initial state
    tenant_svc
        .create(tenant_id.clone(), "Snap Tenant".to_owned())
        .await
        .unwrap();
    workspace_svc
        .create(tenant_id.clone(), workspace_id, "WS".to_owned())
        .await
        .unwrap();
    project_svc
        .create(project.clone(), "Proj".to_owned())
        .await
        .unwrap();
    session_svc
        .create(&project, session_id.clone())
        .await
        .unwrap();
    run_svc
        .start(&project, &session_id, run_id.clone(), None)
        .await
        .unwrap();
    task_svc
        .submit(
            &project,
            TaskId::new("task_snap"),
            Some(run_id.clone()),
            None,
            0,
        )
        .await
        .unwrap();

    // Verify initial run state
    let run_before = RunReadModel::get(store.as_ref(), &run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run_before.state,
        RunState::Pending,
        "run should start Pending"
    );

    // Create snapshot — captures current tenant events
    let snapshot = store.create_snapshot(&tenant_id);
    assert!(!snapshot.snapshot_id.is_empty());
    assert!(!snapshot.state_hash.is_empty());
    assert!(snapshot.event_position > 0);

    // Destroy state: transition run to Failed
    let fail_evt: EventEnvelope<RuntimeEvent> = EventEnvelope::for_runtime_event(
        EventId::new("evt_snap_fail"),
        EventSource::Runtime,
        RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: Some(RunState::Pending),
                to: RunState::Failed,
            },
            failure_class: Some(FailureClass::ExecutionError),
            pause_reason: None,
            resume_trigger: None,
        }),
    );
    store.append(&[fail_evt]).await.unwrap();

    // Confirm run is now Failed
    let run_failed = RunReadModel::get(store.as_ref(), &run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run_failed.state,
        RunState::Failed,
        "run should be Failed after deliberate transition"
    );

    // Restore from snapshot
    let report = store.restore_from_snapshot(&snapshot);
    assert!(
        report["events_before"].as_u64().unwrap_or(0) > 0,
        "should report events before restore"
    );
    assert!(
        report["events_after"].as_u64().unwrap_or(0) > 0,
        "should retain snapshot events"
    );

    // Assert run is back to original Pending state
    let run_restored = RunReadModel::get(store.as_ref(), &run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run_restored.state,
        RunState::Pending,
        "run should be restored to Pending after snapshot restore"
    );

    // The task should also be restored
    use cairn_store::projections::TaskReadModel;
    let task_restored = TaskReadModel::get(store.as_ref(), &TaskId::new("task_snap"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        task_restored.state,
        TaskState::Queued,
        "task should be back to Queued"
    );
}
