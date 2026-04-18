#![cfg(feature = "in-memory-runtime")]

//! RFC 005 — external worker lifecycle end-to-end integration tests.
//!
//! Tests the full arc an external worker executes:
//!   1. Register with capability descriptor
//!   2. Claim a task from the queue
//!   3. Send a task-lease heartbeat to extend the hold
//!   4. Report incremental progress
//!   5. Complete the task with results
//!   6. Verify worker health tracking throughout

use std::sync::Arc;

use cairn_domain::lifecycle::TaskState;
use cairn_domain::workers::{ExternalWorkerOutcome, ExternalWorkerProgress, ExternalWorkerReport};
use cairn_domain::{ProjectKey, TaskId, TenantId, WorkerId};
use cairn_runtime::services::{parse_outcome, ExternalWorkerServiceImpl, TaskServiceImpl};
use cairn_runtime::tasks::TaskService;
use cairn_runtime::ExternalWorkerService;
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("t_ew", "w_ew", "p_ew")
}

fn tenant() -> TenantId {
    TenantId::new("t_ew")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Test 1: full happy-path lifecycle ────────────────────────────────────────

/// RFC 005: external worker registers, claims a task, heartbeats, reports
/// progress, then completes — verify every state transition and health update.
///
/// Note: the `register` API does not carry a structured capabilities list;
/// capability metadata is conveyed via the `display_name` (e.g. JSON-encoded
/// tags).  The test documents this gap while verifying the rest of the flow.
#[tokio::test]
async fn external_worker_claim_progress_complete_lifecycle() {
    let store = Arc::new(InMemoryStore::new());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());
    let task_svc = TaskServiceImpl::new(store.clone());

    let worker_id = WorkerId::new("ew_worker_1");
    let task_id = TaskId::new("ew_task_1");

    // ── Step 1: Register with capability descriptor ────────────────────────
    // capabilities are embedded in display_name as the service has no dedicated field.
    let display_name = r#"TextEmbedder v2 [capabilities: embed, summarise, classify]"#.to_owned();

    let registered = worker_svc
        .register(tenant(), worker_id.clone(), display_name.clone())
        .await
        .unwrap();

    assert_eq!(registered.worker_id, worker_id, "worker_id must round-trip");
    assert_eq!(registered.tenant_id, tenant());
    assert_eq!(registered.display_name, display_name);
    assert_eq!(registered.status, "active");
    // No heartbeat yet — worker is registered but not alive.
    assert!(
        !registered.health.is_alive,
        "newly registered worker must not be marked alive"
    );
    assert_eq!(registered.health.last_heartbeat_ms, 0);
    assert_eq!(registered.current_task_id, None);

    // get() must return the same record.
    let fetched = worker_svc.get(&worker_id).await.unwrap().unwrap();
    assert_eq!(fetched.worker_id, worker_id);
    assert_eq!(fetched.display_name, display_name);

    // ── Step 2: Submit a task and claim it ────────────────────────────────
    task_svc
        .submit(&project(), task_id.clone(), None, None, 0)
        .await
        .unwrap();

    let claimed = task_svc
        .claim(&task_id, worker_id.as_str().to_owned(), 30_000)
        .await
        .unwrap();

    assert_eq!(
        claimed.state,
        TaskState::Leased,
        "task must be Leased after claim"
    );
    assert_eq!(
        claimed.lease_owner.as_deref(),
        Some(worker_id.as_str()),
        "lease_owner must equal the claiming worker"
    );
    assert!(
        claimed.lease_expires_at.is_some(),
        "lease_expires_at must be set"
    );

    let lease_token = claimed.version; // token is task.version after the claim event

    // ── Step 3: Extend the lease via heartbeat ────────────────────────────
    let before_hb = claimed.lease_expires_at.unwrap();
    // Small sleep not needed — just check the heartbeat event is accepted.
    let after_hb_task = task_svc.heartbeat(&task_id, 60_000).await.unwrap();

    assert_eq!(
        after_hb_task.state,
        TaskState::Leased,
        "heartbeat must not change state"
    );
    let new_expiry = after_hb_task.lease_expires_at.unwrap();
    // The new expiry must be at least as far out as the original (or equal if
    // clock resolution collapses the delta in tests).
    assert!(
        new_expiry >= before_hb,
        "heartbeat must not reduce lease expiry; before={before_hb}, after={new_expiry}"
    );

    // ── Step 4: Report incremental progress ───────────────────────────────
    let progress_at = now_ms();
    worker_svc
        .report(ExternalWorkerReport {
            project: project(),
            worker_id: worker_id.clone(),
            run_id: None,
            task_id: task_id.clone(),
            lease_token,
            reported_at_ms: progress_at,
            progress: Some(ExternalWorkerProgress {
                message: Some("Chunk 1/3 embedded".to_owned()),
                percent_milli: Some(3_333), // 33.33%
            }),
            outcome: None,
        })
        .await
        .unwrap();

    // Worker health must reflect the progress report.
    let worker_mid = worker_svc.get(&worker_id).await.unwrap().unwrap();
    assert!(
        worker_mid.health.is_alive,
        "worker must be alive after progress report"
    );
    assert!(
        worker_mid.health.last_heartbeat_ms >= progress_at,
        "last_heartbeat_ms must be >= the report timestamp"
    );
    assert_eq!(
        worker_mid.current_task_id,
        Some(task_id.clone()),
        "current_task_id must be set while task is in progress"
    );

    // Task must still be Leased (progress report does not complete it).
    let mid_task = task_svc.get(&task_id).await.unwrap().unwrap();
    assert_eq!(mid_task.state, TaskState::Leased);

    // Second progress report — 100%.
    worker_svc
        .report(ExternalWorkerReport {
            project: project(),
            worker_id: worker_id.clone(),
            run_id: None,
            task_id: task_id.clone(),
            lease_token,
            reported_at_ms: now_ms(),
            progress: Some(ExternalWorkerProgress {
                message: Some("All chunks embedded".to_owned()),
                percent_milli: Some(10_000), // 100%
            }),
            outcome: None,
        })
        .await
        .unwrap();

    // ── Step 5: Complete the task with results ────────────────────────────
    let complete_at = now_ms();
    worker_svc
        .report(ExternalWorkerReport {
            project: project(),
            worker_id: worker_id.clone(),
            run_id: None,
            task_id: task_id.clone(),
            lease_token,
            reported_at_ms: complete_at,
            progress: None,
            outcome: Some(ExternalWorkerOutcome::Completed),
        })
        .await
        .unwrap();

    // Task must now be in a terminal Completed state.
    let final_task = task_svc.get(&task_id).await.unwrap().unwrap();
    assert_eq!(
        final_task.state,
        TaskState::Completed,
        "task must be Completed after terminal report"
    );

    // ── Step 6: Verify worker health tracking ─────────────────────────────
    let worker_done = worker_svc.get(&worker_id).await.unwrap().unwrap();
    assert!(
        worker_done.health.is_alive,
        "worker must remain alive after task completion"
    );
    assert!(
        worker_done.health.last_heartbeat_ms >= complete_at,
        "last_heartbeat_ms must reflect the completion report timestamp"
    );
    assert_eq!(
        worker_done.current_task_id, None,
        "current_task_id must be cleared after terminal outcome"
    );
}

// ── Test 2: worker reports task failure ──────────────────────────────────────

/// RFC 005: when a worker reports ExternalWorkerOutcome::Failed, the task
/// must transition to Failed with the specified failure class.
#[tokio::test]
async fn external_worker_reports_task_failure() {
    let store = Arc::new(InMemoryStore::new());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());
    let task_svc = TaskServiceImpl::new(store.clone());

    let worker_id = WorkerId::new("ew_worker_fail");
    let task_id = TaskId::new("ew_task_fail");

    worker_svc
        .register(tenant(), worker_id.clone(), "Failing Worker".to_owned())
        .await
        .unwrap();

    task_svc
        .submit(&project(), task_id.clone(), None, None, 0)
        .await
        .unwrap();
    let claimed = task_svc
        .claim(&task_id, worker_id.as_str().to_owned(), 30_000)
        .await
        .unwrap();

    let lease_token = claimed.version;

    worker_svc
        .report(ExternalWorkerReport {
            project: project(),
            worker_id: worker_id.clone(),
            run_id: None,
            task_id: task_id.clone(),
            lease_token,
            reported_at_ms: now_ms(),
            progress: None,
            outcome: Some(ExternalWorkerOutcome::Failed {
                failure_class: cairn_domain::lifecycle::FailureClass::ExecutionError,
            }),
        })
        .await
        .unwrap();

    let failed_task = task_svc.get(&task_id).await.unwrap().unwrap();
    assert_eq!(
        failed_task.state,
        TaskState::Failed,
        "task must be Failed after worker failure report"
    );
    assert_eq!(
        failed_task.failure_class,
        Some(cairn_domain::lifecycle::FailureClass::ExecutionError),
        "failure class must propagate from the worker report"
    );

    // Worker health: current_task_id must be cleared.
    let worker = worker_svc.get(&worker_id).await.unwrap().unwrap();
    assert_eq!(
        worker.current_task_id, None,
        "current_task_id must be cleared after task failure"
    );
}

// ── Test 3: reporting against a terminal task is rejected ────────────────────

/// RFC 005: once a task reaches a terminal state, further worker reports must
/// be rejected with an InvalidTransition error (idempotency guard).
#[tokio::test]
async fn reporting_on_terminal_task_returns_error() {
    let store = Arc::new(InMemoryStore::new());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());
    let task_svc = TaskServiceImpl::new(store.clone());

    let worker_id = WorkerId::new("ew_worker_term");
    let task_id = TaskId::new("ew_task_term");

    worker_svc
        .register(tenant(), worker_id.clone(), "Worker".to_owned())
        .await
        .unwrap();

    task_svc
        .submit(&project(), task_id.clone(), None, None, 0)
        .await
        .unwrap();
    let claimed = task_svc
        .claim(&task_id, worker_id.as_str().to_owned(), 30_000)
        .await
        .unwrap();
    let lease_token = claimed.version;

    // Complete the task.
    worker_svc
        .report(ExternalWorkerReport {
            project: project(),
            worker_id: worker_id.clone(),
            run_id: None,
            task_id: task_id.clone(),
            lease_token,
            reported_at_ms: now_ms(),
            progress: None,
            outcome: Some(ExternalWorkerOutcome::Completed),
        })
        .await
        .unwrap();

    // A second report on the now-terminal task must fail.
    let err = worker_svc
        .report(ExternalWorkerReport {
            project: project(),
            worker_id: worker_id.clone(),
            run_id: None,
            task_id: task_id.clone(),
            lease_token,
            reported_at_ms: now_ms(),
            progress: None,
            outcome: Some(ExternalWorkerOutcome::Completed),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            cairn_runtime::error::RuntimeError::InvalidTransition { .. }
        ),
        "RFC 005: report on terminal task must return InvalidTransition; got: {err:?}"
    );
}

// ── Test 4: worker suspend and reactivate ─────────────────────────────────────

/// RFC 005: a worker can be suspended (taking it offline) and later reactivated.
/// Status transitions must be reflected in the read model.
#[tokio::test]
async fn worker_suspend_and_reactivate() {
    let store = Arc::new(InMemoryStore::new());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());

    let worker_id = WorkerId::new("ew_worker_suspend");

    worker_svc
        .register(tenant(), worker_id.clone(), "Suspendable Worker".to_owned())
        .await
        .unwrap();

    // Baseline: active.
    let active = worker_svc.get(&worker_id).await.unwrap().unwrap();
    assert_eq!(active.status, "active");

    // Suspend.
    let suspended = worker_svc.suspend(&worker_id).await.unwrap();
    assert_eq!(
        suspended.status, "suspended",
        "worker status must be 'suspended' after suspend"
    );

    // Reactivate.
    let reactivated = worker_svc.reactivate(&worker_id).await.unwrap();
    assert_eq!(
        reactivated.status, "active",
        "worker status must be 'active' after reactivate"
    );

    // Read model must reflect the final state.
    let final_record = worker_svc.get(&worker_id).await.unwrap().unwrap();
    assert_eq!(final_record.status, "active");
}

// ── Test 5: multiple workers listed by tenant ─────────────────────────────────

/// RFC 005: list_by_tenant returns all workers registered for a tenant,
/// in registration order, with correct pagination.
#[tokio::test]
async fn list_workers_by_tenant() {
    let store = Arc::new(InMemoryStore::new());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());

    let worker_a = WorkerId::new("ew_list_a");
    let worker_b = WorkerId::new("ew_list_b");
    let worker_c = WorkerId::new("ew_list_c");

    // Register three workers for the test tenant.
    for (id, name) in [
        (worker_a.clone(), "Alpha Worker"),
        (worker_b.clone(), "Beta Worker"),
        (worker_c.clone(), "Gamma Worker"),
    ] {
        worker_svc
            .register(tenant(), id, name.to_owned())
            .await
            .unwrap();
    }

    // Register a worker for a different tenant — must not appear in the list.
    worker_svc
        .register(
            TenantId::new("other_tenant"),
            WorkerId::new("ew_other"),
            "Other Tenant Worker".to_owned(),
        )
        .await
        .unwrap();

    // List all workers for the test tenant.
    let all = worker_svc.list(&tenant(), 10, 0).await.unwrap();
    assert_eq!(
        all.len(),
        3,
        "list must return exactly the 3 registered workers"
    );

    let ids: Vec<&WorkerId> = all.iter().map(|w| &w.worker_id).collect();
    assert!(ids.contains(&&worker_a));
    assert!(ids.contains(&&worker_b));
    assert!(ids.contains(&&worker_c));

    // Pagination: offset=1, limit=2 must skip the first.
    let page = worker_svc.list(&tenant(), 2, 1).await.unwrap();
    assert_eq!(
        page.len(),
        2,
        "paginated list must respect limit and offset"
    );

    // Pagination: offset=10 must return empty.
    let empty = worker_svc.list(&tenant(), 10, 10).await.unwrap();
    assert!(empty.is_empty(), "offset past end must return empty list");
}

// ── Test 6: parse_outcome helper ────────────────────────────────────────────

/// RFC 005: the parse_outcome helper converts API strings to domain outcomes.
/// Unknown strings must return an error (fail-safe).
#[test]
fn parse_outcome_converts_valid_strings() {
    // Valid outcomes.
    assert!(matches!(
        parse_outcome("completed"),
        Ok(ExternalWorkerOutcome::Completed)
    ));
    assert!(matches!(
        parse_outcome("canceled"),
        Ok(ExternalWorkerOutcome::Canceled)
    ));
    assert!(matches!(
        parse_outcome("failed"),
        Ok(ExternalWorkerOutcome::Failed { .. })
    ));
    assert!(matches!(
        parse_outcome("suspended"),
        Ok(ExternalWorkerOutcome::Suspended { .. })
    ));

    // Unknown string must fail.
    assert!(
        parse_outcome("unknown_value").is_err(),
        "unknown outcome string must return Err"
    );
}
