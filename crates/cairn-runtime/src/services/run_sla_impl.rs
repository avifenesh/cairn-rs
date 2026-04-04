//! Run SLA service implementation per RFC 005.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::sla::{SlaConfig, SlaBreach, SlaStatus};
use cairn_domain::*;
use cairn_store::projections::{RunReadModel, RunSlaReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::run_sla::RunSlaService;

pub struct RunSlaServiceImpl<S> {
    store: Arc<S>,
}

impl<S> RunSlaServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<S> RunSlaService for RunSlaServiceImpl<S>
where
    S: EventLog + RunSlaReadModel + RunReadModel + Send + Sync + 'static,
{
    async fn set_sla(
        &self,
        run_id: RunId,
        tenant_id: TenantId,
        target_ms: u64,
        alert_pct: u8,
    ) -> Result<SlaConfig, RuntimeError> {
        let set_at_ms = now_ms();
        let event = make_envelope(RuntimeEvent::RunSlaSet(RunSlaSet {
            run_id: run_id.clone(),
            tenant_id: tenant_id.clone(),
            target_completion_ms: target_ms,
            alert_at_percent: alert_pct,
            set_at_ms,
        }));
        self.store.append(&[event]).await?;

        RunSlaReadModel::get_sla(self.store.as_ref(), &run_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("SLA config not found after set".to_owned()))
    }

    async fn check_sla(&self, run_id: &RunId) -> Result<SlaStatus, RuntimeError> {
        let config = RunSlaReadModel::get_sla(self.store.as_ref(), run_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "sla_config",
                id: run_id.to_string(),
            })?;

        let run = RunReadModel::get(self.store.as_ref(), run_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "run",
                id: run_id.to_string(),
            })?;

        let now = now_ms();
        let elapsed_ms = now.saturating_sub(run.created_at);
        let target_ms = config.target_completion_ms;
        let percent_used = if target_ms == 0 {
            u64::MAX
        } else {
            elapsed_ms.saturating_mul(100) / target_ms
        };

        Ok(SlaStatus {
            on_track: elapsed_ms < target_ms,
            elapsed_ms,
            target_ms,
            percent_used,
        })
    }

    async fn check_and_breach(&self, run_id: &RunId) -> Result<bool, RuntimeError> {
        let status = self.check_sla(run_id).await?;
        if status.on_track {
            return Ok(false);
        }

        // Already breached — check if we already emitted the event
        let existing = RunSlaReadModel::get_breach(self.store.as_ref(), run_id).await?;
        if existing.is_some() {
            return Ok(false); // idempotent
        }

        let config = RunSlaReadModel::get_sla(self.store.as_ref(), run_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "sla_config",
                id: run_id.to_string(),
            })?;

        let event = make_envelope(RuntimeEvent::RunSlaBreached(RunSlaBreached {
            run_id: run_id.clone(),
            tenant_id: config.tenant_id.clone(),
            elapsed_ms: status.elapsed_ms,
            target_ms: status.target_ms,
            breached_at_ms: now_ms(),
        }));
        self.store.append(&[event]).await?;
        Ok(true)
    }

    async fn get_sla(&self, run_id: &RunId) -> Result<Option<SlaConfig>, RuntimeError> {
        Ok(RunSlaReadModel::get_sla(self.store.as_ref(), run_id).await?)
    }

    async fn list_breached_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<SlaBreach>, RuntimeError> {
        Ok(RunSlaReadModel::list_breached_by_tenant(self.store.as_ref(), tenant_id).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_store::InMemoryStore;

    #[tokio::test]
    async fn run_sla_percent_used_exceeds_100_when_target_elapsed() {
        use tokio::time::{sleep, Duration};

        let store = Arc::new(InMemoryStore::new());
        let service = RunSlaServiceImpl::new(store.clone());

        // Create a run in the store
        let project = ProjectKey::new("t", "w", "p");
        let run_id = RunId::new("run_sla_test");
        let session_id = SessionId::new("sess_sla");
        let tenant_id = TenantId::new("t");

        store
            .append(&[make_envelope(RuntimeEvent::RunCreated(RunCreated {
                project: project.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                parent_run_id: None,
                prompt_release_id: None,
            }))])
            .await
            .unwrap();

        // Set SLA of 50ms
        service
            .set_sla(run_id.clone(), tenant_id.clone(), 50, 80)
            .await
            .unwrap();

        // Wait longer than the target
        sleep(Duration::from_millis(100)).await;

        // check_sla should show percent_used > 100
        let status = service.check_sla(&run_id).await.unwrap();
        assert!(
            status.percent_used > 100,
            "percent_used should exceed 100 after target elapsed, got {}",
            status.percent_used
        );
        assert!(!status.on_track, "run should not be on track");
        assert_eq!(status.target_ms, 50);

        // check_and_breach should emit RunSlaBreached
        let breached = service.check_and_breach(&run_id).await.unwrap();
        assert!(breached, "should have emitted RunSlaBreached");

        // Verify event in log
        let events = store.read_stream(None, 100).await.unwrap();
        let sla_breached = events
            .iter()
            .any(|e| matches!(&e.envelope.payload, RuntimeEvent::RunSlaBreached(ev) if ev.run_id == run_id));
        assert!(sla_breached, "RunSlaBreached event must be in event log");

        // list_breached_by_tenant returns the run
        let breaches = service
            .list_breached_by_tenant(&tenant_id)
            .await
            .unwrap();
        assert_eq!(breaches.len(), 1);
        assert_eq!(breaches[0].run_id, run_id);

        // Idempotent — second call returns false
        let second = service.check_and_breach(&run_id).await.unwrap();
        assert!(!second, "second check_and_breach must be idempotent");
    }
}
