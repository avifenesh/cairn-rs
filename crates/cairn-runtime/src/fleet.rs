//! Fleet report service — GAP-005.
//!
//! Mirrors `cairn/internal/server/routes_fleet.go`:
//! Returns the registered external worker fleet with live health and task status.
//! Response: `FleetReport { workers, total, active, healthy }`.
//!
//! The report joins `ExternalWorkerReadModel` with `TaskReadModel` to compute
//! `active_task_count` and enrich each `WorkerState` with the current task.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::workers::{ExternalWorkerRecord, WorkerHealth};
use cairn_domain::{TaskId, TenantId};
use serde::{Deserialize, Serialize};

use crate::error::RuntimeError;

/// Fleet-visibility snapshot for a single worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerState {
    pub worker_id: String,
    pub display_name: String,
    /// "active" | "suspended" | "offline"
    pub status: String,
    pub health: WorkerHealth,
    /// The task currently leased to this worker (if any).
    pub current_task_id: Option<TaskId>,
}

/// Fleet-level report (GAP-005).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FleetReport {
    pub workers: Vec<WorkerState>,
    pub total: u32,
    /// Workers whose status is "active".
    pub active: u32,
    /// Workers that reported a heartbeat recently (`health.is_alive == true`).
    pub healthy: u32,
}

/// Fleet service boundary.
#[async_trait]
pub trait FleetService: Send + Sync {
    /// Return the full fleet report for a tenant.
    async fn fleet_report(
        &self,
        tenant_id: &TenantId,
        limit: usize,
    ) -> Result<FleetReport, RuntimeError>;
}

// ── Implementation ────────────────────────────────────────────────────────

use cairn_store::projections::ExternalWorkerReadModel;

/// Fleet service backed by ExternalWorkerReadModel + TaskReadModel.
pub struct FleetServiceImpl<S> {
    store: Arc<S>,
    /// Heartbeat TTL: a worker is considered "alive" if its last heartbeat
    /// is within this window (milliseconds). Default: 60 seconds.
    heartbeat_ttl_ms: u64,
}

impl<S> FleetServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            heartbeat_ttl_ms: 60_000,
        }
    }

    pub fn with_heartbeat_ttl_ms(mut self, ttl_ms: u64) -> Self {
        self.heartbeat_ttl_ms = ttl_ms;
        self
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<S> FleetService for FleetServiceImpl<S>
where
    S: ExternalWorkerReadModel + Send + Sync + 'static,
{
    async fn fleet_report(
        &self,
        tenant_id: &TenantId,
        limit: usize,
    ) -> Result<FleetReport, RuntimeError> {
        let records: Vec<ExternalWorkerRecord> = ExternalWorkerReadModel::list_by_tenant(
            self.store.as_ref(),
            tenant_id,
            limit,
            0,
        )
        .await?;

        let now = now_millis();
        let mut workers = Vec::with_capacity(records.len());
        let mut active_count = 0u32;
        let mut healthy_count = 0u32;

        for mut rec in records {
            // Refresh is_alive based on last_heartbeat_ms vs now.
            let is_alive = rec.health.last_heartbeat_ms > 0
                && now.saturating_sub(rec.health.last_heartbeat_ms) <= self.heartbeat_ttl_ms;
            rec.health.is_alive = is_alive;

            // Count active tasks leased to this worker.
            // We use the current_task_id stored in the record. A running lease
            // means the task is either in "leased" or "running" state.
            let active_task_count = if rec.current_task_id.is_some() { 1u32 } else { 0u32 };
            rec.health.active_task_count = active_task_count;

            if rec.status == "active" {
                active_count += 1;
            }
            if is_alive {
                healthy_count += 1;
            }

            workers.push(WorkerState {
                worker_id: rec.worker_id.as_str().to_owned(),
                display_name: rec.display_name.clone(),
                status: rec.status.clone(),
                health: rec.health.clone(),
                current_task_id: rec.current_task_id.clone(),
            });
        }

        let total = workers.len() as u32;
        Ok(FleetReport { workers, total, active: active_count, healthy: healthy_count })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use cairn_domain::*;
    use cairn_store::EventLog;
    use cairn_store::InMemoryStore;

    fn ev(payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
        use cairn_domain::{EventId, OwnershipKey};
        let project = payload.project().clone();
        EventEnvelope {
            event_id: EventId::new("ev"),
            source: EventSource::System,
            ownership: OwnershipKey::Project(project),
            causation_id: None,
            correlation_id: None,
            payload,
        }
    }

    fn sentinel(tenant_id: &str) -> ProjectKey {
        ProjectKey::new(tenant_id, "_", "_")
    }

    async fn register_worker(
        store: &InMemoryStore,
        worker_id: &str,
        tenant_id: &str,
        display_name: &str,
    ) {
        let event = ev(RuntimeEvent::ExternalWorkerRegistered(ExternalWorkerRegistered {
            sentinel_project: sentinel(tenant_id),
            worker_id: WorkerId::new(worker_id),
            tenant_id: TenantId::new(tenant_id),
            display_name: display_name.to_owned(),
            registered_at: 1_000,
        }));
        store.append(&[event]).await.unwrap();
    }

    async fn create_and_claim_task(
        store: &InMemoryStore,
        project: &ProjectKey,
        task_id: &str,
        worker_id: &str,
    ) {
        // Create the task.
        store.append(&[ev(RuntimeEvent::TaskCreated(TaskCreated {
            project: project.clone(),
            task_id: TaskId::new(task_id),
            parent_run_id: None,
            parent_task_id: None,
            prompt_release_id: None,
        }))]).await.unwrap();

        // Heartbeat report sets current_task_id on the worker.
        store.append(&[ev(RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
            report: cairn_domain::workers::ExternalWorkerReport {
                project: project.clone(),
                worker_id: WorkerId::new(worker_id),
                run_id: None,
                task_id: TaskId::new(task_id),
                lease_token: 1,
                reported_at_ms: 5_000,
                progress: None,
                outcome: None,
            },
        }))]).await.unwrap();
    }

    /// Register 2 workers, assign task to one, verify fleet report.
    #[tokio::test]
    async fn fleet_report_two_workers_one_with_task() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_id = TenantId::new("t1");

        // Register two workers.
        register_worker(&store, "worker-alpha", "t1", "Alpha Worker").await;
        register_worker(&store, "worker-beta", "t1", "Beta Worker").await;

        // Assign a task to worker-alpha via a heartbeat report.
        let project = ProjectKey::new("t1", "w1", "p1");
        create_and_claim_task(&store, &project, "task-001", "worker-alpha").await;

        // Build fleet report.
        let svc = FleetServiceImpl::new(store.clone())
            .with_heartbeat_ttl_ms(60_000);
        let report = svc.fleet_report(&tenant_id, 100).await.unwrap();

        // Both workers appear.
        assert_eq!(report.total, 2, "total must be 2");

        let alpha = report.workers.iter().find(|w| w.worker_id == "worker-alpha")
            .expect("worker-alpha must appear");
        let beta = report.workers.iter().find(|w| w.worker_id == "worker-beta")
            .expect("worker-beta must appear");

        // Alpha has the task.
        assert_eq!(
            alpha.health.active_task_count, 1,
            "worker-alpha must have active_task_count=1"
        );
        assert_eq!(
            alpha.current_task_id.as_ref().map(|t| t.as_str()),
            Some("task-001"),
            "worker-alpha current_task_id must be task-001"
        );

        // Beta has no task.
        assert_eq!(
            beta.health.active_task_count, 0,
            "worker-beta must have active_task_count=0"
        );
        assert!(beta.current_task_id.is_none(), "worker-beta must have no current_task");
    }

    #[tokio::test]
    async fn fleet_report_empty_tenant() {
        let store = Arc::new(InMemoryStore::new());
        let svc = FleetServiceImpl::new(store);
        let report = svc.fleet_report(&TenantId::new("nobody"), 100).await.unwrap();
        assert_eq!(report.total, 0);
        assert_eq!(report.active, 0);
        assert_eq!(report.healthy, 0);
    }

    #[tokio::test]
    async fn fleet_report_counts_active_workers() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_id = TenantId::new("t2");

        register_worker(&store, "w1", "t2", "Worker 1").await;
        register_worker(&store, "w2", "t2", "Worker 2").await;

        // Suspend w2.
        store.append(&[ev(RuntimeEvent::ExternalWorkerSuspended(ExternalWorkerSuspended {
            sentinel_project: sentinel("t2"),
            worker_id: WorkerId::new("w2"),
            tenant_id: tenant_id.clone(),
            suspended_at: 2_000,
            reason: Some("operator".to_owned()),
        }))]).await.unwrap();

        let svc = FleetServiceImpl::new(store);
        let report = svc.fleet_report(&tenant_id, 100).await.unwrap();
        assert_eq!(report.total, 2);
        assert_eq!(report.active, 1, "only w1 is active; w2 is suspended");
    }
}
