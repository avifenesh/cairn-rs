// Run and task lifecycle integration tests.
//
// These tests create real FF executions in Valkey. Each test uses unique IDs
// to avoid cross-test interference. However, keys accumulate — use a dedicated
// test Valkey instance and FLUSHDB between full test runs if needed.
//
// Terminal operations (complete/fail/cancel) require the execution to be
// Active (leased). run_service.start() creates executions in Waiting state.
// We use task_service.submit() + task_service.claim() to get an Active
// execution, then call terminal operations on the task.

use cairn_domain::lifecycle::{FailureClass, TaskState};
use cairn_domain::TaskId;

use crate::TestHarness;

#[tokio::test]
#[ignore]
async fn test_create_and_read_run() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    let record = h
        .fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("start failed");

    assert_eq!(record.run_id, run_id);
    assert_eq!(record.project.tenant_id, h.project.tenant_id);
    assert_eq!(record.project.workspace_id, h.project.workspace_id);
    assert_eq!(record.project.project_id, h.project.project_id);

    let fetched = h
        .fabric
        .runs
        .get(&h.project, &run_id)
        .await
        .expect("get failed")
        .expect("run not found");

    assert_eq!(fetched.run_id, run_id);
    assert_eq!(fetched.project.tenant_id, h.project.tenant_id);

    h.teardown().await;
}

#[tokio::test]
#[ignore]
async fn test_tags_readable() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    let record = h
        .fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("start failed");

    assert_eq!(record.session_id, session_id);

    h.teardown().await;
}

#[tokio::test]
#[ignore]
async fn test_duplicate_start_is_idempotent() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    let first = h
        .fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("first start failed");

    let second = h
        .fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("second start failed");

    assert_eq!(first.run_id, second.run_id);
    assert_eq!(first.state, second.state);
    assert_eq!(first.project, second.project);
    assert_eq!(first.session_id, second.session_id);

    h.teardown().await;
}

// Terminal operations require Active (leased) execution.
// We use task_service which creates + claims in the correct flow.

#[tokio::test]
#[ignore]
async fn test_complete_task() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = TaskId::new(format!("integ_task_{}", uuid::Uuid::new_v4()));

    h.fabric
        .tasks
        .submit(
            &h.project,
            task_id.clone(),
            None,
            None,
            0,
            Some(&session_id),
        )
        .await
        .expect("submit failed");

    h.fabric
        .tasks
        .claim(&h.project, &task_id, "test-worker".into(), 30_000)
        .await
        .expect("claim failed");

    let completed = h
        .fabric
        .tasks
        .complete(&h.project, &task_id)
        .await
        .expect("complete failed");

    assert_eq!(completed.state, TaskState::Completed);

    let fetched = h
        .fabric
        .tasks
        .get(&h.project, &task_id)
        .await
        .expect("get failed")
        .expect("task not found");

    assert_eq!(fetched.state, TaskState::Completed);

    h.teardown().await;
}

#[tokio::test]
#[ignore]
async fn test_fail_task_terminal() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = TaskId::new(format!("integ_task_{}", uuid::Uuid::new_v4()));

    h.fabric
        .tasks
        .submit(
            &h.project,
            task_id.clone(),
            None,
            None,
            0,
            Some(&session_id),
        )
        .await
        .expect("submit failed");

    h.fabric
        .tasks
        .claim(&h.project, &task_id, "test-worker".into(), 30_000)
        .await
        .expect("claim failed");

    let failed = h
        .fabric
        .tasks
        .fail(&h.project, &task_id, FailureClass::ExecutionError)
        .await
        .expect("fail failed");

    // task_service submits with max_retries=2, so first fail triggers retry.
    // State will be Queued (delayed backoff mapped to Pending/Queued in cairn).
    assert!(
        matches!(failed.state, TaskState::Queued | TaskState::Failed),
        "expected Queued (retry backoff) or Failed (terminal), got {:?}",
        failed.state
    );

    h.teardown().await;
}

#[tokio::test]
#[ignore]
async fn test_cancel_task() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = TaskId::new(format!("integ_task_{}", uuid::Uuid::new_v4()));

    h.fabric
        .tasks
        .submit(
            &h.project,
            task_id.clone(),
            None,
            None,
            0,
            Some(&session_id),
        )
        .await
        .expect("submit failed");

    h.fabric
        .tasks
        .claim(&h.project, &task_id, "test-worker".into(), 30_000)
        .await
        .expect("claim failed");

    let cancelled = h
        .fabric
        .tasks
        .cancel(&h.project, &task_id)
        .await
        .expect("cancel failed");

    assert_eq!(cancelled.state, TaskState::Canceled);

    h.teardown().await;
}
