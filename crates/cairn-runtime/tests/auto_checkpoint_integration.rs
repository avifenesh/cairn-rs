use std::sync::Arc;

use cairn_domain::{CheckpointStrategy, ProjectKey, RunId, SessionId, TaskId};
use cairn_runtime::{
    RunService, RunServiceImpl, SessionService, SessionServiceImpl, TaskService, TaskServiceImpl,
};
use cairn_store::projections::CheckpointReadModel;
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("tenant_auto_cp", "ws_auto_cp", "project_auto_cp")
}

#[tokio::test]
async fn auto_checkpoint_saves_checkpoint_when_triggered_on_task_complete() {
    let store = Arc::new(InMemoryStore::new());
    let sessions = SessionServiceImpl::new(store.clone());
    let runs = RunServiceImpl::new(store.clone());
    let tasks = TaskServiceImpl::new(store.clone());
    let project = project();
    let session_id = SessionId::new("session_auto_cp");
    let run_id = RunId::new("run_auto_cp");

    sessions.create(&project, session_id.clone()).await.unwrap();
    runs.start(&project, &session_id, run_id.clone(), None)
        .await
        .unwrap();
    runs.set_checkpoint_strategy(&run_id, "auto_60s".to_owned())
        .await
        .unwrap();

    tasks
        .submit(
            &project,
            TaskId::new("task_auto_cp"),
            Some(run_id.clone()),
            None,
            0,
        )
        .await
        .unwrap();
    tasks
        .claim(
            &TaskId::new("task_auto_cp"),
            "worker_auto_cp".to_owned(),
            60_000,
        )
        .await
        .unwrap();
    tasks.start(&TaskId::new("task_auto_cp")).await.unwrap();
    tasks.complete(&TaskId::new("task_auto_cp")).await.unwrap();

    let checkpoints = CheckpointReadModel::list_by_run(store.as_ref(), &run_id, 10)
        .await
        .unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert!(checkpoints[0]
        .checkpoint_id
        .as_str()
        .starts_with("cp_auto_run_auto_cp_task_auto_cp_"));
}
