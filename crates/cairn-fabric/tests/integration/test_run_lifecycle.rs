use cairn_domain::lifecycle::{FailureClass, RunState};

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
async fn test_complete_run() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    h.fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("start failed");

    let completed = h
        .fabric
        .runs
        .complete(&h.project, &run_id)
        .await
        .expect("complete failed");

    assert_eq!(completed.state, RunState::Completed);

    let fetched = h
        .fabric
        .runs
        .get(&h.project, &run_id)
        .await
        .expect("get failed")
        .expect("run not found");

    assert_eq!(fetched.state, RunState::Completed);

    h.teardown().await;
}

#[tokio::test]
#[ignore]
async fn test_fail_run_terminal() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    h.fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("start failed");

    let failed = h
        .fabric
        .runs
        .fail(&h.project, &run_id, FailureClass::ExecutionError)
        .await
        .expect("fail failed");

    assert_eq!(failed.state, RunState::Failed);

    h.teardown().await;
}

#[tokio::test]
#[ignore]
async fn test_cancel_run() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    h.fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("start failed");

    let cancelled = h
        .fabric
        .runs
        .cancel(&h.project, &run_id)
        .await
        .expect("cancel failed");

    assert_eq!(cancelled.state, RunState::Canceled);

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

    h.teardown().await;
}
