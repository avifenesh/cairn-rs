use std::sync::Arc;

use cairn_domain::{ProjectKey, TaskId, TaskState};
use cairn_runtime::{TaskService, TaskServiceImpl};
use cairn_store::projections::TaskDependencyReadModel;
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("tenant_dep", "ws_dep", "project_dep")
}

#[tokio::test]
async fn task_dependency_resolves_when_prerequisite_completes() {
    let store = Arc::new(InMemoryStore::new());
    let tasks = TaskServiceImpl::new(store.clone());
    let project = project();

    tasks
        .submit(&project, TaskId::new("task_a"), None, None, 0)
        .await
        .unwrap();
    tasks
        .submit(&project, TaskId::new("task_b"), None, None, 0)
        .await
        .unwrap();

    let dependency = tasks
        .declare_dependency(&TaskId::new("task_b"), &TaskId::new("task_a"))
        .await
        .unwrap();
    assert_eq!(dependency.resolved_at_ms, None);

    let task_b = tasks.get(&TaskId::new("task_b")).await.unwrap().unwrap();
    assert_eq!(task_b.state, TaskState::WaitingDependency);

    tasks
        .claim(&TaskId::new("task_a"), "worker-a".to_owned(), 60_000)
        .await
        .unwrap();
    tasks.start(&TaskId::new("task_a")).await.unwrap();
    tasks.complete(&TaskId::new("task_a")).await.unwrap();

    let deps = TaskDependencyReadModel::list_blocking(store.as_ref(), &TaskId::new("task_b"))
        .await
        .unwrap();
    assert_eq!(deps.len(), 1);
    assert!(deps[0].resolved_at_ms.is_some());

    let unresolved = tasks
        .check_dependencies(&TaskId::new("task_b"))
        .await
        .unwrap();
    assert!(unresolved.is_empty());

    let task_b = tasks.get(&TaskId::new("task_b")).await.unwrap().unwrap();
    assert_eq!(task_b.state, TaskState::Queued);
}
