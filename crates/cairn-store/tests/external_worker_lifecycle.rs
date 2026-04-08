//! External worker lifecycle tests (RFC 011).
//!
//! Validates the full external worker pipeline: registration, heartbeat
//! (ExternalWorkerReported), suspension, and reactivation.
//!
//! ExternalWorkerRecord state machine:
//!   ExternalWorkerRegistered  → status="active",    health.is_alive=false
//!   ExternalWorkerReported    → health.is_alive=true, last_heartbeat_ms set
//!                               current_task_id set (cleared when outcome present)
//!   ExternalWorkerSuspended   → status="suspended"
//!   ExternalWorkerReactivated → status="active"
//!
//! Sentinel project: worker events are tenant-scoped (no real project) so they
//! carry a sentinel ProjectKey("tenant", "_", "_") per the domain contract.

use cairn_domain::workers::{ExternalWorkerOutcome, ExternalWorkerReport};
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RuntimeEvent, TaskId, TenantId,
    WorkerId, WorkspaceId,
};
use cairn_domain::{
    ExternalWorkerReactivated, ExternalWorkerRegistered, ExternalWorkerReported,
    ExternalWorkerSuspended,
};
use cairn_store::{projections::ExternalWorkerReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Sentinel project key for tenant-scoped worker events (no real project).
fn sentinel(tenant: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new(tenant),
        workspace_id: WorkspaceId::new("_"),
        project_id: ProjectId::new("_"),
    }
}

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn register(
    evt_id: &str,
    worker_id: &str,
    tenant: &str,
    display_name: &str,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::ExternalWorkerRegistered(ExternalWorkerRegistered {
            sentinel_project: sentinel(tenant),
            worker_id: WorkerId::new(worker_id),
            tenant_id: TenantId::new(tenant),
            display_name: display_name.to_owned(),
            registered_at: ts,
        }),
    )
}

fn heartbeat(
    evt_id: &str,
    worker_id: &str,
    tenant: &str,
    task_id: &str,
    reported_at_ms: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
            report: ExternalWorkerReport {
                project: sentinel(tenant),
                worker_id: WorkerId::new(worker_id),
                run_id: None,
                task_id: TaskId::new(task_id),
                lease_token: 1,
                reported_at_ms,
                progress: None,
                outcome: None,
            },
        }),
    )
}

fn heartbeat_with_outcome(
    evt_id: &str,
    worker_id: &str,
    tenant: &str,
    task_id: &str,
    reported_at_ms: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
            report: ExternalWorkerReport {
                project: sentinel(tenant),
                worker_id: WorkerId::new(worker_id),
                run_id: None,
                task_id: TaskId::new(task_id),
                lease_token: 1,
                reported_at_ms,
                progress: None,
                outcome: Some(ExternalWorkerOutcome::Completed),
            },
        }),
    )
}

fn suspend(
    evt_id: &str,
    worker_id: &str,
    tenant: &str,
    reason: Option<&str>,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::ExternalWorkerSuspended(ExternalWorkerSuspended {
            sentinel_project: sentinel(tenant),
            worker_id: WorkerId::new(worker_id),
            tenant_id: TenantId::new(tenant),
            suspended_at: ts,
            reason: reason.map(str::to_owned),
        }),
    )
}

fn reactivate(evt_id: &str, worker_id: &str, tenant: &str, ts: u64) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::ExternalWorkerReactivated(ExternalWorkerReactivated {
            sentinel_project: sentinel(tenant),
            worker_id: WorkerId::new(worker_id),
            tenant_id: TenantId::new(tenant),
            reactivated_at: ts,
        }),
    )
}

// ── 1. ExternalWorkerRegistered → record stored with initial state ─────────────

#[tokio::test]
async fn worker_registered_stores_record_with_active_status() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let worker_id = WorkerId::new("worker_001");

    store
        .append(&[register("e1", "worker_001", "t_worker", "Build Bot", ts)])
        .await
        .unwrap();

    let record = ExternalWorkerReadModel::get(&store, &worker_id)
        .await
        .unwrap()
        .expect("ExternalWorkerRecord must exist after ExternalWorkerRegistered");

    assert_eq!(record.worker_id, worker_id);
    assert_eq!(record.tenant_id.as_str(), "t_worker");
    assert_eq!(record.display_name, "Build Bot");
    assert_eq!(record.status, "active", "newly registered worker is active");
    assert_eq!(record.registered_at, ts);
    assert!(
        !record.health.is_alive,
        "freshly registered worker has not yet heartbeated"
    );
    assert_eq!(
        record.health.last_heartbeat_ms, 0,
        "no heartbeat yet → last_heartbeat_ms=0"
    );
    assert!(record.current_task_id.is_none());
}

// ── 2. ExternalWorkerReported (heartbeat) → health updated ────────────────────

#[tokio::test]
async fn worker_reported_sets_health_alive_and_heartbeat() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[register("e1", "worker_hb", "t_hb", "Heartbeat Bot", ts)])
        .await
        .unwrap();

    // Verify not alive before heartbeat.
    let before = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_hb"))
        .await
        .unwrap()
        .unwrap();
    assert!(!before.health.is_alive);

    // Send heartbeat.
    store
        .append(&[heartbeat("e2", "worker_hb", "t_hb", "task_001", ts + 5_000)])
        .await
        .unwrap();

    let after = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_hb"))
        .await
        .unwrap()
        .unwrap();

    assert!(
        after.health.is_alive,
        "health.is_alive=true after heartbeat"
    );
    assert_eq!(
        after.health.last_heartbeat_ms,
        ts + 5_000,
        "last_heartbeat_ms must reflect the reported_at_ms"
    );
    assert_eq!(
        after.current_task_id,
        Some(TaskId::new("task_001")),
        "current_task_id set from the report's task_id"
    );
}

// ── 3. Multiple heartbeats advance last_heartbeat_ms ──────────────────────────

#[tokio::test]
async fn multiple_heartbeats_advance_last_heartbeat_ms() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[register("e1", "worker_mhb", "t_mhb", "Multi-HB Bot", ts)])
        .await
        .unwrap();

    for i in 1u64..=3 {
        store
            .append(&[heartbeat(
                "e_hb",
                "worker_mhb",
                "t_mhb",
                "task_x",
                ts + i * 10_000,
            )])
            .await
            .unwrap();

        let r = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_mhb"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            r.health.last_heartbeat_ms,
            ts + i * 10_000,
            "heartbeat {i}: last_heartbeat_ms must advance"
        );
        assert!(r.health.is_alive);
    }
}

// ── 4. Heartbeat with outcome clears current_task_id ──────────────────────────

#[tokio::test]
async fn heartbeat_with_outcome_clears_current_task() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            register("e1", "worker_fin", "t_fin", "Finisher Bot", ts),
            heartbeat("e2", "worker_fin", "t_fin", "task_final", ts + 1_000),
        ])
        .await
        .unwrap();

    // current_task_id is set after heartbeat.
    let active = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_fin"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active.current_task_id, Some(TaskId::new("task_final")));

    // Terminal report (outcome present) clears current_task_id.
    store
        .append(&[heartbeat_with_outcome(
            "e3",
            "worker_fin",
            "t_fin",
            "task_final",
            ts + 5_000,
        )])
        .await
        .unwrap();

    let finished = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_fin"))
        .await
        .unwrap()
        .unwrap();
    assert!(
        finished.current_task_id.is_none(),
        "current_task_id cleared when outcome is present in report"
    );
    assert!(
        finished.health.is_alive,
        "health.is_alive remains true even after terminal report"
    );
}

// ── 5. ExternalWorkerSuspended → status = "suspended" ─────────────────────────

#[tokio::test]
async fn worker_suspended_sets_suspended_status() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            register("e1", "worker_sus", "t_sus", "Suspension Bot", ts),
            heartbeat("e2", "worker_sus", "t_sus", "task_sus", ts + 1_000),
            suspend(
                "e3",
                "worker_sus",
                "t_sus",
                Some("exceeded error threshold"),
                ts + 2_000,
            ),
        ])
        .await
        .unwrap();

    let record = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_sus"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        record.status, "suspended",
        "ExternalWorkerSuspended must set status=suspended"
    );
    // health.is_alive stays true (suspension doesn't reset health).
    assert!(
        record.health.is_alive,
        "health.is_alive remains true after suspension (health != status)"
    );
}

// ── 6. ExternalWorkerReactivated → status = "active" again ────────────────────

#[tokio::test]
async fn worker_reactivated_restores_active_status() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            register("e1", "worker_rea", "t_rea", "Reactivation Bot", ts),
            suspend("e2", "worker_rea", "t_rea", None, ts + 1_000),
        ])
        .await
        .unwrap();

    let suspended = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_rea"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(suspended.status, "suspended");

    store
        .append(&[reactivate("e3", "worker_rea", "t_rea", ts + 2_000)])
        .await
        .unwrap();

    let active = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_rea"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        active.status, "active",
        "ExternalWorkerReactivated must restore status=active"
    );
}

// ── 7. Full lifecycle: Register → Heartbeat → Suspend → Reactivate ────────────

#[tokio::test]
async fn full_lifecycle_register_heartbeat_suspend_reactivate() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // 1. Register.
    store
        .append(&[register(
            "e1",
            "worker_full",
            "t_full",
            "Full Lifecycle Bot",
            ts,
        )])
        .await
        .unwrap();
    let r = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_full"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, "active");
    assert!(!r.health.is_alive);

    // 2. Heartbeat.
    store
        .append(&[heartbeat(
            "e2",
            "worker_full",
            "t_full",
            "task_lc_1",
            ts + 1_000,
        )])
        .await
        .unwrap();
    let r = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_full"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, "active");
    assert!(r.health.is_alive);
    assert_eq!(r.health.last_heartbeat_ms, ts + 1_000);

    // 3. Suspend.
    store
        .append(&[suspend(
            "e3",
            "worker_full",
            "t_full",
            Some("maintenance window"),
            ts + 2_000,
        )])
        .await
        .unwrap();
    let r = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_full"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, "suspended");

    // 4. Reactivate.
    store
        .append(&[reactivate("e4", "worker_full", "t_full", ts + 3_000)])
        .await
        .unwrap();
    let r = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_full"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, "active");
    // health.is_alive persists from the last heartbeat.
    assert!(r.health.is_alive);
}

// ── 8. list_by_tenant scoping ─────────────────────────────────────────────────

#[tokio::test]
async fn list_by_tenant_returns_only_tenant_workers() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            register("e1", "w_t1_a", "tenant_one", "Bot A", ts),
            register("e2", "w_t1_b", "tenant_one", "Bot B", ts + 1),
            register("e3", "w_t2_a", "tenant_two", "Bot C", ts + 2),
        ])
        .await
        .unwrap();

    let t1_workers =
        ExternalWorkerReadModel::list_by_tenant(&store, &TenantId::new("tenant_one"), 10, 0)
            .await
            .unwrap();
    assert_eq!(t1_workers.len(), 2, "tenant_one has 2 workers");
    assert!(t1_workers
        .iter()
        .all(|w| w.tenant_id.as_str() == "tenant_one"));
    let t1_ids: Vec<_> = t1_workers.iter().map(|w| w.worker_id.as_str()).collect();
    assert!(t1_ids.contains(&"w_t1_a"));
    assert!(t1_ids.contains(&"w_t1_b"));
    assert!(
        !t1_ids.contains(&"w_t2_a"),
        "tenant_two worker must not appear in tenant_one list"
    );

    let t2_workers =
        ExternalWorkerReadModel::list_by_tenant(&store, &TenantId::new("tenant_two"), 10, 0)
            .await
            .unwrap();
    assert_eq!(t2_workers.len(), 1);
    assert_eq!(t2_workers[0].worker_id.as_str(), "w_t2_a");
    assert_eq!(t2_workers[0].display_name, "Bot C");

    // Unknown tenant returns empty.
    let empty =
        ExternalWorkerReadModel::list_by_tenant(&store, &TenantId::new("tenant_unknown"), 10, 0)
            .await
            .unwrap();
    assert!(empty.is_empty());
}

// ── 9. list_by_tenant pagination ─────────────────────────────────────────────

#[tokio::test]
async fn list_by_tenant_respects_limit_and_offset() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    for i in 0u32..4 {
        store
            .append(&[register(
                &format!("e{i}"),
                &format!("w_page_{i:02}"),
                "t_page",
                &format!("Worker {i}"),
                ts + i as u64,
            )])
            .await
            .unwrap();
    }

    // All 4 workers registered.
    let all = ExternalWorkerReadModel::list_by_tenant(&store, &TenantId::new("t_page"), 10, 0)
        .await
        .unwrap();
    assert_eq!(all.len(), 4);

    // First 2.
    let page1 = ExternalWorkerReadModel::list_by_tenant(&store, &TenantId::new("t_page"), 2, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);

    // Skip 3, get last 1.
    let page2 = ExternalWorkerReadModel::list_by_tenant(&store, &TenantId::new("t_page"), 10, 3)
        .await
        .unwrap();
    assert_eq!(page2.len(), 1);
}

// ── 10. get() returns None for unregistered worker ────────────────────────────

#[tokio::test]
async fn get_returns_none_for_unknown_worker() {
    let store = InMemoryStore::new();
    let result = ExternalWorkerReadModel::get(&store, &WorkerId::new("ghost_worker"))
        .await
        .unwrap();
    assert!(result.is_none());
}

// ── 11. Suspend then suspend again is idempotent ──────────────────────────────

#[tokio::test]
async fn double_suspend_remains_suspended() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            register("e1", "worker_dd", "t_dd", "Double Bot", ts),
            suspend("e2", "worker_dd", "t_dd", Some("first suspension"), ts + 1),
            suspend("e3", "worker_dd", "t_dd", Some("second suspension"), ts + 2),
        ])
        .await
        .unwrap();

    let r = ExternalWorkerReadModel::get(&store, &WorkerId::new("worker_dd"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, "suspended", "repeated suspension is idempotent");
}
