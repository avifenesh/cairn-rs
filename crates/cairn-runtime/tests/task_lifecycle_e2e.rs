//! RFC 005 task lifecycle end-to-end integration test.
//!
//! Validates the complete task state machine:
//!   (1) create session + run
//!   (2) submit task → Queued
//!   (3) claim → Leased, lease_owner set
//!   (4) heartbeat → lease extended
//!   (5) start → Running, complete → Completed
//!   (6) second task: claim, verify list_expired_leases reports it when
//!       querying with a far-future `now_ms` (deterministic lease expiry test)

use std::sync::Arc;

use cairn_domain::{
    FailureClass, ProjectKey, RunId, SessionId, TaskId, TaskState,
};
use cairn_runtime::{
    RunService, RunServiceImpl, SessionService, SessionServiceImpl,
    TaskService, TaskServiceImpl,
};
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("t_task", "ws_task", "proj_task")
}

fn services() -> (
    Arc<InMemoryStore>,
    SessionServiceImpl<InMemoryStore>,
    RunServiceImpl<InMemoryStore>,
    TaskServiceImpl<InMemoryStore>,
) {
    let store = Arc::new(InMemoryStore::new());
    let sessions = SessionServiceImpl::new(store.clone());
    let runs = RunServiceImpl::new(store.clone());
    let tasks = TaskServiceImpl::new(store.clone());
    (store, sessions, runs, tasks)
}

// ── Step-by-step tests ────────────────────────────────────────────────────

/// (1) Create a session and run as execution context.
#[tokio::test]
async fn step1_create_session_and_run() {
    let (_, sessions, runs, _) = services();
    let sess_id = SessionId::new("sess_lc1");
    let run_id = RunId::new("run_lc1");

    let session = sessions.create(&project(), sess_id.clone()).await.unwrap();
    assert_eq!(session.session_id, sess_id);

    let run = runs
        .start(&project(), &sess_id, run_id.clone(), None)
        .await
        .unwrap();
    assert_eq!(run.run_id, run_id);
}

/// (2) Submit a task — initial state must be Queued.
#[tokio::test]
async fn step2_submit_task_starts_queued() {
    let (_, _, _, tasks) = services();
    let task_id = TaskId::new("task_lc2");

    let record = tasks
        .submit(&project(), task_id.clone(), None, None, 0)
        .await
        .unwrap();

    assert_eq!(record.task_id, task_id);
    assert_eq!(record.state, TaskState::Queued, "submitted task must start Queued");
    assert!(record.lease_owner.is_none());
    assert!(record.lease_expires_at.is_none());
}

/// (3) Claim a task — state moves to Leased, lease fields are set.
#[tokio::test]
async fn step3_claim_moves_to_leased() {
    let (_, _, _, tasks) = services();
    let task_id = TaskId::new("task_lc3");

    tasks.submit(&project(), task_id.clone(), None, None, 0).await.unwrap();

    let leased = tasks
        .claim(&task_id, "worker_01".to_owned(), 60_000)
        .await
        .unwrap();

    assert_eq!(leased.state, TaskState::Leased, "claimed task must be Leased");
    assert_eq!(
        leased.lease_owner.as_ref().map(|w| w.as_str()),
        Some("worker_01"),
        "lease_owner must be set to the claiming worker"
    );
    assert!(leased.lease_expires_at.is_some(), "lease_expires_at must be set");

    // Claiming an already-leased task must fail.
    let double_claim = tasks.claim(&task_id, "other_worker".to_owned(), 60_000).await;
    assert!(double_claim.is_err(), "double-claim on a Leased task must be rejected");
}

/// (4) Heartbeat extends the lease expiry.
#[tokio::test]
async fn step4_heartbeat_extends_lease() {
    let (_, _, _, tasks) = services();
    let task_id = TaskId::new("task_lc4");

    tasks.submit(&project(), task_id.clone(), None, None, 0).await.unwrap();
    let leased = tasks.claim(&task_id, "worker_01".to_owned(), 60_000).await.unwrap();
    let original_expiry = leased.lease_expires_at.unwrap();

    // Heartbeat with a large extension to guarantee the new expiry is strictly later.
    let after_hb = tasks.heartbeat(&task_id, 120_000).await.unwrap();

    assert!(
        after_hb.lease_expires_at.unwrap() > original_expiry,
        "heartbeat must produce a later lease_expires_at"
    );

    // Heartbeat on a Queued task (not leased) must fail.
    let task2 = TaskId::new("task_lc4b");
    tasks.submit(&project(), task2.clone(), None, None, 0).await.unwrap();
    let queued_hb = tasks.heartbeat(&task2, 60_000).await;
    assert!(queued_hb.is_err(), "heartbeat on a Queued task must be rejected");
}

/// (5) Start → Running, then complete → Completed.
#[tokio::test]
async fn step5_start_then_complete() {
    let (_, _, _, tasks) = services();
    let task_id = TaskId::new("task_lc5");

    tasks.submit(&project(), task_id.clone(), None, None, 0).await.unwrap();
    tasks.claim(&task_id, "worker_01".to_owned(), 60_000).await.unwrap();

    let running = tasks.start(&task_id).await.unwrap();
    assert_eq!(running.state, TaskState::Running, "start must move task to Running");

    let completed = tasks.complete(&task_id).await.unwrap();
    assert_eq!(completed.state, TaskState::Completed, "complete must move task to Completed");
    assert!(completed.state.is_terminal());

    // Completing an already-terminal task must fail.
    let double_complete = tasks.complete(&task_id).await;
    assert!(double_complete.is_err(), "double-complete must be rejected");
}

/// (6) Claim a task then verify list_expired_leases reports it when a
///     far-future `now_ms` is used — no real sleep needed.
#[tokio::test]
async fn step6_lease_expiry_detection() {
    let (_, _, _, tasks) = services();
    let task_id = TaskId::new("task_lc6");

    tasks.submit(&project(), task_id.clone(), None, None, 0).await.unwrap();

    // Claim with a 30-second lease.
    let leased = tasks.claim(&task_id, "worker_01".to_owned(), 30_000).await.unwrap();
    assert_eq!(leased.state, TaskState::Leased);
    let expiry = leased.lease_expires_at.expect("lease_expires_at must be set");

    // Simulated far future — all active leases appear expired.
    let far_future = expiry + 1;
    let expired = tasks.list_expired_leases(far_future, 100).await.unwrap();
    assert!(
        expired.iter().any(|t| t.task_id == task_id),
        "task_lc6 must appear in expired leases when queried past its expiry"
    );

    // Before the lease expires the task must NOT appear.
    let before_expiry = expiry.saturating_sub(1);
    let not_expired = tasks.list_expired_leases(before_expiry, 100).await.unwrap();
    assert!(
        !not_expired.iter().any(|t| t.task_id == task_id),
        "task_lc6 must NOT appear in expired leases before its expiry"
    );
}

// ── Full sequential lifecycle ─────────────────────────────────────────────

/// All six steps in a single shared-store test.
#[tokio::test]
async fn full_task_lifecycle() {
    let (_, sessions, runs, tasks) = services();

    // (1) Session + run.
    let sess_id = SessionId::new("sess_full");
    let run_id = RunId::new("run_full");
    sessions.create(&project(), sess_id.clone()).await.unwrap();
    let run = runs.start(&project(), &sess_id, run_id.clone(), None).await.unwrap();
    assert_eq!(run.run_id, run_id);

    // (2) Submit → Queued.
    let task_id = TaskId::new("task_full_1");
    let queued = tasks
        .submit(&project(), task_id.clone(), Some(run_id.clone()), None, 0)
        .await
        .unwrap();
    assert_eq!(queued.state, TaskState::Queued);
    assert_eq!(queued.parent_run_id.as_ref(), Some(&run_id));

    // (3) Claim → Leased.
    let leased = tasks.claim(&task_id, "worker_01".to_owned(), 60_000).await.unwrap();
    assert_eq!(leased.state, TaskState::Leased);
    assert_eq!(
        leased.lease_owner.as_ref().map(|w| w.as_str()),
        Some("worker_01")
    );
    let original_expiry = leased.lease_expires_at.unwrap();

    // (4) Heartbeat → extends expiry.
    let after_hb = tasks.heartbeat(&task_id, 120_000).await.unwrap();
    assert!(
        after_hb.lease_expires_at.unwrap() > original_expiry,
        "heartbeat must extend lease"
    );

    // (5) Start → Running, complete → Completed.
    let running = tasks.start(&task_id).await.unwrap();
    assert_eq!(running.state, TaskState::Running);

    let completed = tasks.complete(&task_id).await.unwrap();
    assert_eq!(completed.state, TaskState::Completed);
    assert!(completed.state.is_terminal());

    // (6) Second task: claim and verify expiry detection.
    let task2_id = TaskId::new("task_full_2");
    tasks.submit(&project(), task2_id.clone(), Some(run_id), None, 0).await.unwrap();
    let leased2 = tasks.claim(&task2_id, "worker_01".to_owned(), 5_000).await.unwrap();
    let expiry2 = leased2.lease_expires_at.unwrap();

    let expired = tasks.list_expired_leases(expiry2 + 1, 100).await.unwrap();
    assert!(
        expired.iter().any(|t| t.task_id == task2_id),
        "task_full_2 must appear as expired when queried past its lease"
    );
}

// ── Edge-case tests ───────────────────────────────────────────────────────

/// cancel() in Queued state moves to Canceled (terminal).
#[tokio::test]
async fn cancel_queued_task() {
    let (_, _, _, tasks) = services();
    let task_id = TaskId::new("task_cancel");
    tasks.submit(&project(), task_id.clone(), None, None, 0).await.unwrap();

    let canceled = tasks.cancel(&task_id).await.unwrap();
    assert_eq!(canceled.state, TaskState::Canceled);
    assert!(canceled.state.is_terminal());
}

/// fail() with LeaseExpired yields RetryableFailed; with ExecutionError yields Failed.
#[tokio::test]
async fn fail_task_uses_correct_failure_class() {
    let (_, _, _, tasks) = services();

    // Retryable failure.
    let t1 = TaskId::new("task_fail_retry");
    tasks.submit(&project(), t1.clone(), None, None, 0).await.unwrap();
    tasks.claim(&t1, "worker_01".to_owned(), 60_000).await.unwrap();
    let retryable = tasks.fail(&t1, FailureClass::LeaseExpired).await.unwrap();
    assert_eq!(retryable.state, TaskState::RetryableFailed);
    assert!(!retryable.state.is_terminal());

    // Non-retryable failure.
    let t2 = TaskId::new("task_fail_terminal");
    tasks.submit(&project(), t2.clone(), None, None, 0).await.unwrap();
    tasks.claim(&t2, "worker_01".to_owned(), 60_000).await.unwrap();
    let failed = tasks.fail(&t2, FailureClass::ExecutionError).await.unwrap();
    assert_eq!(failed.state, TaskState::Failed);
    assert!(failed.state.is_terminal());
}

/// dead_letter() and list_dead_lettered roundtrip.
#[tokio::test]
async fn dead_letter_and_query() {
    let (_, _, _, tasks) = services();
    let task_id = TaskId::new("task_dlq");

    tasks.submit(&project(), task_id.clone(), None, None, 0).await.unwrap();
    let dl = tasks.dead_letter(&task_id).await.unwrap();
    assert_eq!(dl.state, TaskState::DeadLettered);

    let dlq = tasks.list_dead_lettered(&project(), 100, 0).await.unwrap();
    assert!(
        dlq.iter().any(|t| t.task_id == task_id),
        "dead-lettered task must appear in list_dead_lettered"
    );
}
