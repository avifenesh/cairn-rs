//! RFC 011 external worker fleet integration tests.
//!
//! Validates the fleet monitoring pipeline through InMemoryStore:
//! - ExternalWorkerRegistered events populate the fleet read-model.
//! - ExternalWorkerReported (heartbeat) marks a worker healthy.
//! - ExternalWorkerSuspended transitions a worker to "suspended".
//! - ExternalWorkerReactivated restores a worker to "active".
//! - Fleet listing returns workers ordered by registration time.

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ExternalWorkerReactivated, ExternalWorkerRegistered,
    ExternalWorkerReported, ExternalWorkerSuspended, ProjectKey, RuntimeEvent, TaskId,
    TenantId, WorkerId,
};
use cairn_domain::workers::{ExternalWorkerProgress, ExternalWorkerReport};
use cairn_store::{projections::ExternalWorkerReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn tenant_id() -> TenantId {
    TenantId::new("tenant_fleet")
}

/// Sentinel project key used for tenant-scoped worker events (no real project).
fn sentinel() -> ProjectKey {
    ProjectKey::new("tenant_fleet", "_", "_")
}

fn worker_id(n: u8) -> WorkerId {
    WorkerId::new(format!("worker_{n}"))
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(id),
        EventSource::Runtime,
        payload.into(),
    )
}

fn register_event(n: u8) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_reg_{n}"),
        RuntimeEvent::ExternalWorkerRegistered(ExternalWorkerRegistered {
            sentinel_project: sentinel(),
            worker_id: worker_id(n),
            tenant_id: tenant_id(),
            display_name: format!("Worker {n}"),
            registered_at: (n as u64) * 1_000,
        }),
    )
}

fn heartbeat_event(n: u8, reported_at_ms: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_hb_{n}"),
        RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
            report: ExternalWorkerReport {
                project: sentinel(),
                worker_id: worker_id(n),
                run_id: None,
                task_id: TaskId::new(format!("task_for_{n}")),
                lease_token: 1,
                reported_at_ms,
                progress: Some(ExternalWorkerProgress {
                    message: Some("alive".to_owned()),
                    percent_milli: Some(0),
                }),
                outcome: None, // heartbeat — no terminal outcome
            },
        }),
    )
}

fn suspend_event(n: u8) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_suspend_{n}"),
        RuntimeEvent::ExternalWorkerSuspended(ExternalWorkerSuspended {
            sentinel_project: sentinel(),
            worker_id: worker_id(n),
            tenant_id: tenant_id(),
            suspended_at: 99_000,
            reason: Some("operator-initiated suspension".to_owned()),
        }),
    )
}

fn reactivate_event(n: u8) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_reactivate_{n}"),
        RuntimeEvent::ExternalWorkerReactivated(ExternalWorkerReactivated {
            sentinel_project: sentinel(),
            worker_id: worker_id(n),
            tenant_id: tenant_id(),
            reactivated_at: 200_000,
        }),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) Register 3 workers; (2) fleet listing returns all 3 in registration order.
#[tokio::test]
async fn register_three_workers_all_listed() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[register_event(1), register_event(2), register_event(3)])
        .await
        .unwrap();

    let fleet = ExternalWorkerReadModel::list_by_tenant(
        store.as_ref(),
        &tenant_id(),
        100,
        0,
    )
    .await
    .unwrap();

    assert_eq!(fleet.len(), 3, "all 3 registered workers must appear in the fleet listing");

    // Registration order is preserved (sorted by registered_at).
    assert_eq!(fleet[0].worker_id.as_str(), "worker_1");
    assert_eq!(fleet[1].worker_id.as_str(), "worker_2");
    assert_eq!(fleet[2].worker_id.as_str(), "worker_3");

    // All start as "active" with no heartbeat.
    for w in &fleet {
        assert_eq!(w.status, "active", "freshly registered workers must have status 'active'");
        assert!(!w.health.is_alive, "no heartbeat yet — is_alive must be false");
        assert_eq!(w.health.last_heartbeat_ms, 0, "no heartbeat timestamp yet");
    }
}

/// (3) Heartbeat for worker 1 marks it alive; workers 2 and 3 remain stale.
#[tokio::test]
async fn heartbeat_marks_worker_healthy() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[register_event(1), register_event(2), register_event(3)])
        .await
        .unwrap();

    store
        .append(&[heartbeat_event(1, 50_000)])
        .await
        .unwrap();

    let w1 = ExternalWorkerReadModel::get(store.as_ref(), &worker_id(1))
        .await
        .unwrap()
        .expect("worker 1 must exist");

    assert!(w1.health.is_alive, "worker 1 must be alive after heartbeat");
    assert_eq!(
        w1.health.last_heartbeat_ms, 50_000,
        "last_heartbeat_ms must match the reported_at_ms"
    );

    // Worker 1 should have a current task assigned (outcome=None on heartbeat).
    assert!(
        w1.current_task_id.is_some(),
        "worker 1 must show a current task after heartbeat with no outcome"
    );

    // Workers 2 and 3 remain stale (no heartbeat sent).
    for n in [2u8, 3u8] {
        let w = ExternalWorkerReadModel::get(store.as_ref(), &worker_id(n))
            .await
            .unwrap()
            .unwrap();
        assert!(
            !w.health.is_alive,
            "worker {n} never sent a heartbeat — must not be alive"
        );
    }
}

/// (4) Suspend worker 2; (5) fleet shows worker 1 healthy, worker 2 suspended,
/// worker 3 stale (registered but never sent a heartbeat).
#[tokio::test]
async fn fleet_status_healthy_suspended_stale() {
    let store = Arc::new(InMemoryStore::new());

    // Register all three workers.
    store
        .append(&[register_event(1), register_event(2), register_event(3)])
        .await
        .unwrap();

    // Worker 1: send a heartbeat → healthy.
    store.append(&[heartbeat_event(1, 50_000)]).await.unwrap();

    // Worker 2: suspend.
    store.append(&[suspend_event(2)]).await.unwrap();

    // Worker 3: no further events → stale.

    let w1 = ExternalWorkerReadModel::get(store.as_ref(), &worker_id(1))
        .await
        .unwrap()
        .unwrap();
    let w2 = ExternalWorkerReadModel::get(store.as_ref(), &worker_id(2))
        .await
        .unwrap()
        .unwrap();
    let w3 = ExternalWorkerReadModel::get(store.as_ref(), &worker_id(3))
        .await
        .unwrap()
        .unwrap();

    // Worker 1: healthy — alive and active.
    assert_eq!(w1.status, "active", "worker 1 must be active");
    assert!(w1.health.is_alive, "worker 1 must be alive after heartbeat");

    // Worker 2: suspended — not alive, status "suspended".
    assert_eq!(w2.status, "suspended", "worker 2 must show suspended status");
    assert!(
        !w2.health.is_alive,
        "worker 2 was never given a heartbeat and should not be alive"
    );

    // Worker 3: stale — status still "active" (never explicitly suspended) but no heartbeat.
    assert_eq!(w3.status, "active", "worker 3 retains 'active' status (not explicitly suspended)");
    assert!(
        !w3.health.is_alive,
        "worker 3 is stale — no heartbeat received, is_alive must be false"
    );
    assert_eq!(
        w3.health.last_heartbeat_ms, 0,
        "worker 3 must have zero heartbeat timestamp"
    );

    // Fleet listing summary: 1 alive, 1 suspended, 1 stale.
    let fleet = ExternalWorkerReadModel::list_by_tenant(store.as_ref(), &tenant_id(), 100, 0)
        .await
        .unwrap();
    let alive_count = fleet.iter().filter(|w| w.health.is_alive).count();
    let suspended_count = fleet.iter().filter(|w| w.status == "suspended").count();
    let stale_count = fleet
        .iter()
        .filter(|w| !w.health.is_alive && w.status != "suspended")
        .count();

    assert_eq!(alive_count, 1, "exactly 1 worker should be alive");
    assert_eq!(suspended_count, 1, "exactly 1 worker should be suspended");
    assert_eq!(stale_count, 1, "exactly 1 worker should be stale (active but no heartbeat)");
}

/// (6) Reactivate worker 2 — status must return to "active".
#[tokio::test]
async fn reactivated_worker_returns_to_active() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[register_event(1), register_event(2), register_event(3)])
        .await
        .unwrap();

    // Suspend worker 2.
    store.append(&[suspend_event(2)]).await.unwrap();

    let w2_suspended = ExternalWorkerReadModel::get(store.as_ref(), &worker_id(2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w2_suspended.status, "suspended");

    // Reactivate worker 2.
    store.append(&[reactivate_event(2)]).await.unwrap();

    let w2_active = ExternalWorkerReadModel::get(store.as_ref(), &worker_id(2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        w2_active.status, "active",
        "worker 2 must return to 'active' after reactivation"
    );

    // The reactivation must be in the event log.
    let events = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    let has_reactivated = events.iter().any(|e| {
        matches!(
            &e.envelope.payload,
            RuntimeEvent::ExternalWorkerReactivated(r) if r.worker_id == worker_id(2)
        )
    });
    assert!(
        has_reactivated,
        "ExternalWorkerReactivated event for worker 2 must be in the log"
    );
}

/// Pagination: list_by_tenant with limit and offset works correctly.
#[tokio::test]
async fn fleet_listing_respects_pagination() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[register_event(1), register_event(2), register_event(3)])
        .await
        .unwrap();

    // First page: limit=2, offset=0.
    let page1 = ExternalWorkerReadModel::list_by_tenant(store.as_ref(), &tenant_id(), 2, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].worker_id.as_str(), "worker_1");
    assert_eq!(page1[1].worker_id.as_str(), "worker_2");

    // Second page: limit=2, offset=2.
    let page2 = ExternalWorkerReadModel::list_by_tenant(store.as_ref(), &tenant_id(), 2, 2)
        .await
        .unwrap();
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].worker_id.as_str(), "worker_3");
}

/// Workers from a different tenant must not appear in fleet listing.
#[tokio::test]
async fn fleet_listing_is_tenant_scoped() {
    let store = Arc::new(InMemoryStore::new());

    // Register 2 workers for our tenant and 1 for another.
    store
        .append(&[
            register_event(1),
            register_event(2),
            // Worker for a different tenant.
            ev(
                "evt_reg_other",
                RuntimeEvent::ExternalWorkerRegistered(ExternalWorkerRegistered {
                    sentinel_project: ProjectKey::new("other_tenant", "_", "_"),
                    worker_id: WorkerId::new("worker_other"),
                    tenant_id: TenantId::new("other_tenant"),
                    display_name: "Other Tenant Worker".to_owned(),
                    registered_at: 5_000,
                }),
            ),
        ])
        .await
        .unwrap();

    let fleet = ExternalWorkerReadModel::list_by_tenant(store.as_ref(), &tenant_id(), 100, 0)
        .await
        .unwrap();

    assert_eq!(
        fleet.len(),
        2,
        "fleet listing must only include workers for the specified tenant"
    );
    assert!(
        fleet.iter().all(|w| w.tenant_id == tenant_id()),
        "all listed workers must belong to the queried tenant"
    );
}
