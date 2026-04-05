//! RFC 011 external worker health reporting integration tests.
//!
//! Validates the worker health pipeline through InMemoryStore:
//! - ExternalWorkerRegistered creates a worker record with default health.
//! - ExternalWorkerReported updates last_heartbeat_ms and sets is_alive=true.
//! - current_task_id is set from the report when there is no terminal outcome.
//! - current_task_id is cleared when the report includes a terminal outcome.
//! - Stale workers (no heartbeat) have is_alive=false and heartbeat=0.
//! - list_by_tenant returns all workers with their current health state.

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ExternalWorkerRegistered, ExternalWorkerReported,
    ProjectKey, RuntimeEvent, TaskId, TenantId, WorkerId,
};
use cairn_domain::tenancy::TenantKey;
use cairn_domain::workers::{
    ExternalWorkerOutcome, ExternalWorkerProgress, ExternalWorkerReport,
};
use cairn_domain::lifecycle::FailureClass;
use cairn_store::{projections::ExternalWorkerReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn tenant(n: &str) -> TenantId  { TenantId::new(format!("tenant_w_{n}")) }
fn tenant_key(n: &str) -> TenantKey { TenantKey::new(format!("tenant_w_{n}")) }
fn worker(n: &str) -> WorkerId  { WorkerId::new(format!("worker_{n}")) }
fn project(n: &str) -> ProjectKey { ProjectKey::new(format!("tenant_w_{n}"), "ws", "proj") }
fn task(n: &str) -> TaskId      { TaskId::new(format!("task_{n}")) }

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

fn register_event(n: &str, tenant_n: &str, ts: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_reg_{n}"),
        RuntimeEvent::ExternalWorkerRegistered(ExternalWorkerRegistered {
            sentinel_project: project(tenant_n),
            worker_id: worker(n),
            tenant_id: tenant(tenant_n),
            display_name: format!("Worker {n}"),
            registered_at: ts,
        }),
    )
}

/// Build a heartbeat report (no terminal outcome → worker keeps its task).
fn heartbeat_event(
    worker_n: &str, tenant_n: &str, task_n: &str,
    reported_at: u64, msg: Option<&str>,
) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_hb_{worker_n}_{reported_at}"),
        RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
            report: ExternalWorkerReport {
                project: project(tenant_n),
                worker_id: worker(worker_n),
                run_id: None,
                task_id: task(task_n),
                lease_token: 1,
                reported_at_ms: reported_at,
                progress: msg.map(|m| ExternalWorkerProgress {
                    message: Some(m.to_owned()),
                    percent_milli: None,
                }),
                outcome: None, // ← heartbeat, no terminal outcome
            },
        }),
    )
}

/// Build a terminal report (outcome set → task released).
fn terminal_event(
    worker_n: &str, tenant_n: &str, task_n: &str, ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_terminal_{worker_n}"),
        RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
            report: ExternalWorkerReport {
                project: project(tenant_n),
                worker_id: worker(worker_n),
                run_id: None,
                task_id: task(task_n),
                lease_token: 1,
                reported_at_ms: ts,
                progress: None,
                outcome: Some(ExternalWorkerOutcome::Completed), // ← terminal
            },
        }),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1): Worker registered with default health (is_alive=false, heartbeat=0).
#[tokio::test]
async fn worker_registered_with_default_health() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[register_event("alpha", "1", 1_000)]).await.unwrap();

    let rec = ExternalWorkerReadModel::get(store.as_ref(), &worker("alpha"))
        .await.unwrap()
        .expect("worker must exist after registration");

    assert_eq!(rec.worker_id, worker("alpha"));
    assert_eq!(rec.tenant_id, tenant("1"));
    assert_eq!(rec.status, "active");
    assert!(!rec.health.is_alive, "no heartbeat yet — is_alive must be false");
    assert_eq!(rec.health.last_heartbeat_ms, 0, "no heartbeat yet — timestamp must be 0");
    assert!(rec.current_task_id.is_none(), "no task assigned at registration");
}

/// (2) + (3): ExternalWorkerReported updates last_heartbeat_ms and is_alive=true.
#[tokio::test]
async fn heartbeat_updates_health_fields() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[register_event("beta", "1", 1_000)]).await.unwrap();

    // First heartbeat at t=5000.
    store.append(&[heartbeat_event("beta", "1", "task_1", 5_000, Some("processing chunk 1"))])
        .await.unwrap();

    let after_first = ExternalWorkerReadModel::get(store.as_ref(), &worker("beta"))
        .await.unwrap().unwrap();

    assert!(after_first.health.is_alive, "is_alive must be true after first heartbeat");
    assert_eq!(after_first.health.last_heartbeat_ms, 5_000, "last_heartbeat_ms must be 5000");

    // Second heartbeat at t=10000 — extends the timestamp.
    store.append(&[heartbeat_event("beta", "1", "task_1", 10_000, Some("processing chunk 2"))])
        .await.unwrap();

    let after_second = ExternalWorkerReadModel::get(store.as_ref(), &worker("beta"))
        .await.unwrap().unwrap();

    assert!(after_second.health.is_alive);
    assert_eq!(
        after_second.health.last_heartbeat_ms, 10_000,
        "last_heartbeat_ms must advance to the most recent report"
    );
}

/// (4): current_task_id is set from a heartbeat report (outcome=None)
/// and cleared when a terminal outcome is reported (outcome=Some).
#[tokio::test]
async fn current_task_id_assigned_and_cleared() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[register_event("gamma", "1", 1_000)]).await.unwrap();

    // Heartbeat with task — current_task_id is set.
    store.append(&[heartbeat_event("gamma", "1", "task_x", 5_000, None)])
        .await.unwrap();

    let with_task = ExternalWorkerReadModel::get(store.as_ref(), &worker("gamma"))
        .await.unwrap().unwrap();
    assert_eq!(
        with_task.current_task_id,
        Some(task("task_x")),
        "current_task_id must be set from the heartbeat report"
    );

    // Terminal outcome (Completed) — current_task_id is cleared.
    store.append(&[terminal_event("gamma", "1", "task_x", 8_000)]).await.unwrap();

    let after_terminal = ExternalWorkerReadModel::get(store.as_ref(), &worker("gamma"))
        .await.unwrap().unwrap();
    assert!(
        after_terminal.current_task_id.is_none(),
        "current_task_id must be None after a terminal outcome"
    );
    // Worker is still alive (it reported in with the terminal outcome).
    assert!(after_terminal.health.is_alive, "is_alive stays true after terminal report");
    assert_eq!(after_terminal.health.last_heartbeat_ms, 8_000);
}

/// Terminal outcome: Failed also clears current_task_id.
#[tokio::test]
async fn failed_outcome_clears_current_task_id() {
    let store = Arc::new(InMemoryStore::new());
    store.append(&[register_event("delta", "1", 1_000)]).await.unwrap();
    store.append(&[heartbeat_event("delta", "1", "task_y", 3_000, None)]).await.unwrap();

    // Report failure.
    store.append(&[ev(
        "evt_fail_delta",
        RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
            report: ExternalWorkerReport {
                project: project("1"),
                worker_id: worker("delta"),
                run_id: None,
                task_id: task("task_y"),
                lease_token: 1,
                reported_at_ms: 6_000,
                progress: None,
                outcome: Some(ExternalWorkerOutcome::Failed {
                    failure_class: FailureClass::ExecutionError,
                }),
            },
        }),
    )]).await.unwrap();

    let rec = ExternalWorkerReadModel::get(store.as_ref(), &worker("delta"))
        .await.unwrap().unwrap();
    assert!(rec.current_task_id.is_none(), "Failed outcome must clear current_task_id");
    assert_eq!(rec.health.last_heartbeat_ms, 6_000);
}

/// (5): Stale detection — workers without a recent heartbeat have is_alive=false
/// and last_heartbeat_ms=0 (never sent a heartbeat).
///
/// RFC 011: operators detect stale workers by comparing last_heartbeat_ms against
/// a staleness threshold (e.g. 30 seconds). is_alive is only set to true by the
/// store when a report arrives — it is never automatically reset to false.
#[tokio::test]
async fn stale_detection_workers_without_heartbeat() {
    let store = Arc::new(InMemoryStore::new());

    // Register 3 workers; only worker 2 sends a heartbeat.
    store.append(&[
        register_event("stale_a", "2", 1_000),
        register_event("stale_b", "2", 2_000),
        register_event("stale_c", "2", 3_000),
    ]).await.unwrap();

    // Only stale_b sends a heartbeat.
    store.append(&[heartbeat_event("stale_b", "2", "task_b", 10_000, None)])
        .await.unwrap();

    let workers = ExternalWorkerReadModel::list_by_tenant(
        store.as_ref(), &tenant("2"), 10, 0,
    ).await.unwrap();
    assert_eq!(workers.len(), 3);

    let now_ms = 10_000u64;
    let stale_threshold_ms = 30_000u64; // 30 seconds

    let alive: Vec<_> = workers.iter().filter(|w| w.health.is_alive).collect();
    let stale: Vec<_> = workers.iter().filter(|w| {
        // A worker is "stale" if it has never sent a heartbeat OR last heartbeat
        // is older than the threshold relative to now.
        !w.health.is_alive
            || (w.health.last_heartbeat_ms > 0
                && now_ms.saturating_sub(w.health.last_heartbeat_ms) > stale_threshold_ms)
    }).collect();

    assert_eq!(alive.len(), 1, "only stale_b has sent a heartbeat");
    assert_eq!(alive[0].worker_id, worker("stale_b"));
    assert_eq!(stale.len(), 2, "stale_a and stale_c are stale (no heartbeat)");

    // Stale workers have is_alive=false and last_heartbeat_ms=0.
    for w in &stale {
        assert!(!w.health.is_alive, "stale worker must have is_alive=false");
        assert_eq!(w.health.last_heartbeat_ms, 0, "stale worker must have heartbeat=0");
    }

    // stale_b's heartbeat age is within threshold.
    let alive_worker = &alive[0];
    let age = now_ms.saturating_sub(alive_worker.health.last_heartbeat_ms);
    assert!(
        age <= stale_threshold_ms,
        "stale_b heartbeat age ({age}ms) must be within threshold ({stale_threshold_ms}ms)"
    );
}

/// (6): list_by_tenant returns workers sorted by registered_at;
/// health state is correct per worker.
#[tokio::test]
async fn list_by_tenant_health_ordering() {
    let store = Arc::new(InMemoryStore::new());

    // Register 4 workers at different times.
    for (n, ts) in [("w1", 1_000u64), ("w2", 2_000), ("w3", 3_000), ("w4", 4_000)] {
        store.append(&[register_event(n, "3", ts)]).await.unwrap();
    }

    // Give heartbeats to w1 and w3; leave w2 and w4 stale.
    store.append(&[
        heartbeat_event("w1", "3", "task_a", 10_000, Some("active")),
        heartbeat_event("w3", "3", "task_c", 12_000, Some("active")),
    ]).await.unwrap();

    let workers = ExternalWorkerReadModel::list_by_tenant(
        store.as_ref(), &tenant("3"), 10, 0,
    ).await.unwrap();

    assert_eq!(workers.len(), 4);

    // All workers belong to the correct tenant.
    assert!(workers.iter().all(|w| w.tenant_id == tenant("3")));

    // Workers sorted by registered_at ascending.
    for window in workers.windows(2) {
        assert!(
            window[0].registered_at <= window[1].registered_at,
            "list must be sorted by registered_at: {} <= {}",
            window[0].registered_at, window[1].registered_at
        );
    }

    // Health summary from the listing.
    let alive_count = workers.iter().filter(|w| w.health.is_alive).count();
    let stale_count = workers.iter().filter(|w| !w.health.is_alive).count();
    assert_eq!(alive_count, 2, "w1 and w3 are alive");
    assert_eq!(stale_count, 2, "w2 and w4 are stale");

    // The alive workers have their last_heartbeat_ms set.
    let w1 = workers.iter().find(|w| w.worker_id == worker("w1")).unwrap();
    let w3 = workers.iter().find(|w| w.worker_id == worker("w3")).unwrap();
    assert_eq!(w1.health.last_heartbeat_ms, 10_000);
    assert_eq!(w3.health.last_heartbeat_ms, 12_000);

    // Pagination: limit=2 returns only 2 workers.
    let page1 = ExternalWorkerReadModel::list_by_tenant(
        store.as_ref(), &tenant("3"), 2, 0,
    ).await.unwrap();
    let page2 = ExternalWorkerReadModel::list_by_tenant(
        store.as_ref(), &tenant("3"), 2, 2,
    ).await.unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 2);
    assert_ne!(page1[0].worker_id, page2[0].worker_id, "pages must not overlap");
}
