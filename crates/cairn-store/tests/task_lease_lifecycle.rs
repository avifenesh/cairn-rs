//! RFC 002 task lease lifecycle integration tests.
//!
//! Validates the task leasing pipeline through InMemoryStore:
//! - TaskLeaseClaimed sets lease_owner and lease_expires_at on the task record.
//! - TaskLeaseHeartbeated extends the expiry without changing ownership.
//! - list_expired_leases returns tasks whose TTL has elapsed (detected at read time).
//! - Expired leases can be re-claimed after recovery re-queues the task.
//! - list_by_parent_run returns all tasks for a run including their lease state.

use std::sync::Arc;

use cairn_domain::events::{TaskLeaseClaimed, TaskLeaseHeartbeated, TaskStateChanged};
use cairn_domain::lifecycle::TaskState;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, StateTransition, TaskCreated, TaskId,
};
use cairn_store::{projections::TaskReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_lease", "ws_lease", "proj_lease")
}

fn run_id() -> RunId {
    RunId::new("run_lease_1")
}
fn task_id(n: &str) -> TaskId {
    TaskId::new(format!("task_{n}"))
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

/// Seed session + run + one task.
async fn seed_task(store: &Arc<InMemoryStore>, tid: &str) {
    let sess = SessionId::new(format!("sess_{tid}"));
    store
        .append(&[
            ev(
                &format!("evt_sess_{tid}"),
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: sess.clone(),
                }),
            ),
            ev(
                &format!("evt_run_{tid}"),
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: sess,
                    run_id: run_id(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            ev(
                &format!("evt_task_{tid}"),
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id(tid),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
        ])
        .await
        .unwrap();
}

/// Claim a lease: appends TaskLeaseClaimed + TaskStateChanged(Leased).
async fn claim_lease(
    store: &Arc<InMemoryStore>,
    tid: &str,
    owner: &str,
    lease_token: u64,
    expires_at_ms: u64,
) {
    store
        .append(&[
            ev(
                &format!("evt_claim_{tid}_{lease_token}"),
                RuntimeEvent::TaskLeaseClaimed(TaskLeaseClaimed {
                    project: project(),
                    task_id: task_id(tid),
                    lease_owner: owner.to_owned(),
                    lease_token,
                    lease_expires_at_ms: expires_at_ms,
                }),
            ),
            ev(
                &format!("evt_state_leased_{tid}_{lease_token}"),
                RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project(),
                    task_id: task_id(tid),
                    transition: StateTransition {
                        from: Some(TaskState::Queued),
                        to: TaskState::Leased,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        ])
        .await
        .unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2) + (3): Create session+run+task; claim lease; verify record reflects lease.
#[tokio::test]
async fn task_lease_claimed_updates_record() {
    let store = Arc::new(InMemoryStore::new());
    seed_task(&store, "alpha").await;

    // Task starts Queued with no lease.
    let queued = TaskReadModel::get(store.as_ref(), &task_id("alpha"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued.state, TaskState::Queued);
    assert!(queued.lease_owner.is_none(), "no lease owner before claim");
    assert!(queued.lease_expires_at.is_none(), "no expiry before claim");

    claim_lease(&store, "alpha", "worker_1", 42, 100_000).await;

    let leased = TaskReadModel::get(store.as_ref(), &task_id("alpha"))
        .await
        .unwrap()
        .unwrap();

    // State transitions to Leased.
    assert_eq!(
        leased.state,
        TaskState::Leased,
        "task must be Leased after claim"
    );

    // Lease metadata is set.
    assert_eq!(
        leased.lease_owner.as_deref(),
        Some("worker_1"),
        "lease_owner must be worker_1"
    );
    assert_eq!(
        leased.lease_expires_at,
        Some(100_000),
        "lease_expires_at must be set"
    );

    // Version must have incremented (claimed = 2 events).
    assert!(
        leased.version > queued.version,
        "version must increment after lease events"
    );
}

/// (4): TaskLeaseHeartbeated extends the lease expiry without changing ownership.
#[tokio::test]
async fn task_lease_heartbeat_extends_expiry() {
    let store = Arc::new(InMemoryStore::new());
    seed_task(&store, "beta").await;
    claim_lease(&store, "beta", "worker_2", 99, 50_000).await;

    let after_claim = TaskReadModel::get(store.as_ref(), &task_id("beta"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_claim.lease_expires_at, Some(50_000));
    let version_after_claim = after_claim.version;

    // Heartbeat extends expiry to 90_000ms.
    store
        .append(&[ev(
            "evt_hb_beta",
            RuntimeEvent::TaskLeaseHeartbeated(TaskLeaseHeartbeated {
                project: project(),
                task_id: task_id("beta"),
                lease_token: 99,
                lease_expires_at_ms: 90_000,
            }),
        )])
        .await
        .unwrap();

    let after_hb = TaskReadModel::get(store.as_ref(), &task_id("beta"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        after_hb.lease_expires_at,
        Some(90_000),
        "lease_expires_at must be extended by heartbeat"
    );
    assert_eq!(
        after_hb.lease_owner.as_deref(),
        Some("worker_2"),
        "lease_owner must not change after heartbeat"
    );
    assert!(
        after_hb.version > version_after_claim,
        "version must increment after heartbeat"
    );
}

/// (5): list_expired_leases returns tasks whose TTL has elapsed.
///
/// Expiry is detected at READ TIME — the TTL is checked against the `now`
/// parameter, not via an event projection.
#[tokio::test]
async fn expired_leases_detected_at_read_time() {
    let store = Arc::new(InMemoryStore::new());
    seed_task(&store, "exp").await;

    // Claim with a TTL that has already elapsed (expires_at=1_000, now=5_000).
    claim_lease(&store, "exp", "worker_exp", 7, 1_000).await;

    // At t=5_000 the lease is expired.
    let expired = TaskReadModel::list_expired_leases(store.as_ref(), 5_000, 100)
        .await
        .unwrap();
    assert_eq!(
        expired.len(),
        1,
        "task with past TTL must appear in expired list"
    );
    assert_eq!(expired[0].task_id, task_id("exp"));
    assert_eq!(expired[0].lease_owner.as_deref(), Some("worker_exp"));

    // At t=999 the lease is still valid — no expired tasks.
    let not_expired = TaskReadModel::list_expired_leases(store.as_ref(), 999, 100)
        .await
        .unwrap();
    assert!(
        not_expired.is_empty(),
        "task whose TTL has not elapsed must not appear in expired list"
    );
}

/// (6): After TTL expires, a recovery sweep re-queues the task and it can be
/// re-claimed by a different worker.
#[tokio::test]
async fn expired_lease_allows_reclaim_after_recovery() {
    let store = Arc::new(InMemoryStore::new());
    seed_task(&store, "reclaim").await;

    // Original claim with expired TTL.
    claim_lease(&store, "reclaim", "worker_old", 1, 1_000).await;

    // Verify it appears as expired.
    let expired = TaskReadModel::list_expired_leases(store.as_ref(), 9_999, 100)
        .await
        .unwrap();
    assert_eq!(
        expired.len(),
        1,
        "task must appear in expired list before recovery"
    );

    // Recovery sweep: re-queue the task (clear lease + set state back to Queued).
    store
        .append(&[ev(
            "evt_requeue_reclaim",
            RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: project(),
                task_id: task_id("reclaim"),
                transition: StateTransition {
                    from: Some(TaskState::Leased),
                    to: TaskState::Queued,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
        )])
        .await
        .unwrap();

    // After re-queue the task is Queued again with no lease.
    let requeued = TaskReadModel::get(store.as_ref(), &task_id("reclaim"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        requeued.state,
        TaskState::Queued,
        "task must be Queued after recovery"
    );

    // New worker claims the lease.
    claim_lease(&store, "reclaim", "worker_new", 2, 50_000).await;

    let reclaimed = TaskReadModel::get(store.as_ref(), &task_id("reclaim"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reclaimed.state, TaskState::Leased);
    assert_eq!(
        reclaimed.lease_owner.as_deref(),
        Some("worker_new"),
        "new worker must own the re-claimed lease"
    );
    assert_eq!(reclaimed.lease_expires_at, Some(50_000));

    // No longer appears in the expired list (new TTL is in the future).
    let no_expired = TaskReadModel::list_expired_leases(store.as_ref(), 9_999, 100)
        .await
        .unwrap();
    assert!(
        no_expired.is_empty(),
        "re-claimed task with future TTL must not appear in expired list"
    );
}

/// (7): list_by_parent_run returns all tasks for a run, including their lease state.
#[tokio::test]
async fn list_by_parent_run_returns_tasks_with_lease_state() {
    let store = Arc::new(InMemoryStore::new());

    // Seed session + run once.
    store
        .append(&[
            ev(
                "sess_lr",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new("sess_lr"),
                }),
            ),
            ev(
                "run_lr",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_lr"),
                    run_id: run_id(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            // Three tasks under run_lease_1.
            ev(
                "evt_task_x",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("x"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            ev(
                "evt_task_y",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("y"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            ev(
                "evt_task_z",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("z"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Claim lease on task_x; leave task_y and task_z un-claimed.
    store
        .append(&[
            ev(
                "evt_claim_x",
                RuntimeEvent::TaskLeaseClaimed(TaskLeaseClaimed {
                    project: project(),
                    task_id: task_id("x"),
                    lease_owner: "worker_x".to_owned(),
                    lease_token: 10,
                    lease_expires_at_ms: 80_000,
                }),
            ),
            ev(
                "evt_state_x",
                RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project(),
                    task_id: task_id("x"),
                    transition: StateTransition {
                        from: Some(TaskState::Queued),
                        to: TaskState::Leased,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // list_by_parent_run returns all 3 tasks.
    let tasks = TaskReadModel::list_by_parent_run(store.as_ref(), &run_id(), 100)
        .await
        .unwrap();
    assert_eq!(tasks.len(), 3, "all 3 tasks must be returned for the run");

    // Find task_x in the results.
    let task_x = tasks
        .iter()
        .find(|t| t.task_id == task_id("x"))
        .expect("task_x must be in the list");
    assert_eq!(task_x.state, TaskState::Leased, "task_x must be Leased");
    assert_eq!(
        task_x.lease_owner.as_deref(),
        Some("worker_x"),
        "task_x lease owner correct"
    );
    assert_eq!(task_x.lease_expires_at, Some(80_000));

    // task_y and task_z are Queued with no lease.
    for name in ["y", "z"] {
        let t = tasks.iter().find(|t| t.task_id == task_id(name)).unwrap();
        assert_eq!(t.state, TaskState::Queued, "task_{name} must be Queued");
        assert!(
            t.lease_owner.is_none(),
            "task_{name} must have no lease owner"
        );
    }
}

/// Multiple concurrent leases: different tasks in the same run can be leased
/// by different workers simultaneously.
#[tokio::test]
async fn concurrent_leases_on_different_tasks() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            ev(
                "sess_c",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new("sess_c"),
                }),
            ),
            ev(
                "run_c",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_c"),
                    run_id: run_id(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            ev(
                "task_p",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("p"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            ev(
                "task_q",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("q"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Worker A claims task_p, Worker B claims task_q simultaneously.
    store
        .append(&[
            ev(
                "claim_p",
                RuntimeEvent::TaskLeaseClaimed(TaskLeaseClaimed {
                    project: project(),
                    task_id: task_id("p"),
                    lease_owner: "worker_A".to_owned(),
                    lease_token: 1,
                    lease_expires_at_ms: 60_000,
                }),
            ),
            ev(
                "state_p",
                RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project(),
                    task_id: task_id("p"),
                    transition: StateTransition {
                        from: Some(TaskState::Queued),
                        to: TaskState::Leased,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
            ev(
                "claim_q",
                RuntimeEvent::TaskLeaseClaimed(TaskLeaseClaimed {
                    project: project(),
                    task_id: task_id("q"),
                    lease_owner: "worker_B".to_owned(),
                    lease_token: 2,
                    lease_expires_at_ms: 60_000,
                }),
            ),
            ev(
                "state_q",
                RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project(),
                    task_id: task_id("q"),
                    transition: StateTransition {
                        from: Some(TaskState::Queued),
                        to: TaskState::Leased,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let task_p = TaskReadModel::get(store.as_ref(), &task_id("p"))
        .await
        .unwrap()
        .unwrap();
    let task_q = TaskReadModel::get(store.as_ref(), &task_id("q"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(task_p.lease_owner.as_deref(), Some("worker_A"));
    assert_eq!(task_q.lease_owner.as_deref(), Some("worker_B"));
    assert_ne!(
        task_p.lease_owner, task_q.lease_owner,
        "different workers own different task leases"
    );
}
