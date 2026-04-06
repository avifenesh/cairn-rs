//! RFC 010: run cost alert service implementation.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::providers::RunCostAlert;
use cairn_domain::{
    RunCostAlertSet, RunCostAlertTriggered, RuntimeEvent, RunId, TenantId,
};
use cairn_store::projections::{RunCostAlertReadModel, RunCostReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::run_cost_alerts::RunCostAlertService;

pub struct RunCostAlertServiceImpl<S> {
    store: Arc<S>,
}

impl<S> RunCostAlertServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<S> RunCostAlertService for RunCostAlertServiceImpl<S>
where
    S: EventLog + RunCostReadModel + RunCostAlertReadModel + Send + Sync + 'static,
{
    async fn set_alert(
        &self,
        run_id: RunId,
        tenant_id: TenantId,
        threshold_micros: u64,
    ) -> Result<(), RuntimeError> {
        let event = make_envelope(RuntimeEvent::RunCostAlertSet(RunCostAlertSet {
            run_id,
            tenant_id,
            threshold_micros,
            set_at_ms: now_millis(),
        }));
        self.store.append(&[event]).await?;
        Ok(())
    }

    async fn check_and_trigger(&self, run_id: &RunId) -> Result<bool, RuntimeError> {
        let alert = RunCostAlertReadModel::get_alert(self.store.as_ref(), run_id).await?;

        // If alert already triggered, skip.
        if let Some(ref a) = alert {
            if a.triggered_at_ms > 0 {
                return Ok(false);
            }
        }

        // No threshold set — nothing to do.
        let Some(alert_record) = alert else {
            return Ok(false);
        };

        let cost = RunCostReadModel::get_run_cost(self.store.as_ref(), run_id).await?;
        let total = cost.map(|c| c.total_cost_micros).unwrap_or(0);

        if total >= alert_record.threshold_micros {
            let event = make_envelope(RuntimeEvent::RunCostAlertTriggered(
                RunCostAlertTriggered {
                    run_id: run_id.clone(),
                    tenant_id: alert_record.tenant_id.clone(),
                    threshold_micros: alert_record.threshold_micros,
                    actual_cost_micros: total,
                    triggered_at_ms: now_millis(),
                },
            ));
            self.store.append(&[event]).await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn get_alert(&self, run_id: &RunId) -> Result<Option<RunCostAlert>, RuntimeError> {
        Ok(RunCostAlertReadModel::get_alert(self.store.as_ref(), run_id).await?)
    }

    async fn list_triggered_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<RunCostAlert>, RuntimeError> {
        Ok(
            RunCostAlertReadModel::list_triggered_by_tenant(self.store.as_ref(), tenant_id)
                .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{
        EventEnvelope, EventId, EventSource, RunCostUpdated, SessionId, TenantId,
    };
    use cairn_store::InMemoryStore;

    #[tokio::test]
    async fn run_cost_alert_triggered_when_threshold_exceeded() {
        let store = Arc::new(InMemoryStore::new());
        let svc = RunCostAlertServiceImpl::new(store.clone());
        let run_id = RunId::new("run_alert_1");
        let tenant_id = TenantId::new("tenant_alert");

        // Set an alert threshold of 100 micros.
        svc.set_alert(run_id.clone(), tenant_id.clone(), 100)
            .await
            .unwrap();

        // Accumulate 150 micros cost via RunCostUpdated events.
        store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_cost_1"),
                EventSource::Runtime,
                RuntimeEvent::RunCostUpdated(RunCostUpdated {
                    project: cairn_domain::tenancy::ProjectKey::new("_", "_", "_"),
                    run_id: run_id.clone(),
                    session_id: Some(SessionId::new("sess_alert_1")),
                    tenant_id: Some(tenant_id.clone()),
                    delta_cost_micros: 150,
                    delta_tokens_in: 0,
                    delta_tokens_out: 0,
                    provider_call_id: "call_1".to_owned(),
                    updated_at_ms: 1000,
                }),
            )])
            .await
            .unwrap();

        // Assert RunCostAlertTriggered was derived and stored.
        let events = store.read_stream(None, 100).await.unwrap();
        let triggered = events.iter().any(|e| {
            matches!(
                &e.envelope.payload,
                RuntimeEvent::RunCostAlertTriggered(ev)
                    if ev.run_id == run_id
                        && ev.actual_cost_micros >= 150
                        && ev.threshold_micros == 100
            )
        });
        assert!(triggered, "RunCostAlertTriggered should have been emitted");

        // get_alert should return the triggered alert.
        let alert = svc.get_alert(&run_id).await.unwrap();
        assert!(alert.is_some(), "alert record should exist");
        let alert = alert.unwrap();
        assert_eq!(alert.actual_cost_micros, 150);
        assert_eq!(alert.threshold_micros, 100);
        assert!(alert.triggered_at_ms > 0);

        // list_triggered_by_tenant should include it.
        let alerts = svc.list_triggered_by_tenant(&tenant_id).await.unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].run_id, run_id);
    }

    #[tokio::test]
    async fn run_cost_alert_not_triggered_below_threshold() {
        let store = Arc::new(InMemoryStore::new());
        let svc = RunCostAlertServiceImpl::new(store.clone());
        let run_id = RunId::new("run_alert_low");
        let tenant_id = TenantId::new("tenant_alert_low");

        svc.set_alert(run_id.clone(), tenant_id.clone(), 200)
            .await
            .unwrap();

        store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_cost_low"),
                EventSource::Runtime,
                RuntimeEvent::RunCostUpdated(RunCostUpdated {
                    project: cairn_domain::tenancy::ProjectKey::new("_", "_", "_"),
                    run_id: run_id.clone(),
                    session_id: Some(SessionId::new("sess_alert_low")),
                    tenant_id: Some(tenant_id.clone()),
                    delta_cost_micros: 50,
                    delta_tokens_in: 0,
                    delta_tokens_out: 0,
                    provider_call_id: "call_low".to_owned(),
                    updated_at_ms: 1000,
                }),
            )])
            .await
            .unwrap();

        let events = store.read_stream(None, 100).await.unwrap();
        let triggered = events.iter().any(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::RunCostAlertTriggered(_))
        });
        assert!(!triggered, "alert should NOT fire when cost < threshold");
    }

    #[tokio::test]
    async fn run_cost_alert_fires_only_once() {
        let store = Arc::new(InMemoryStore::new());
        let svc = RunCostAlertServiceImpl::new(store.clone());
        let run_id = RunId::new("run_alert_once");
        let tenant_id = TenantId::new("tenant_alert_once");

        svc.set_alert(run_id.clone(), tenant_id.clone(), 100)
            .await
            .unwrap();

        // Two separate cost updates, each exceeding the threshold.
        for i in 0..2u32 {
            store
                .append(&[EventEnvelope::for_runtime_event(
                    EventId::new(format!("evt_cost_once_{i}")),
                    EventSource::Runtime,
                    RuntimeEvent::RunCostUpdated(RunCostUpdated {
                    project: cairn_domain::tenancy::ProjectKey::new("_", "_", "_"),
                        run_id: run_id.clone(),
                        session_id: Some(SessionId::new("sess_once")),
                        tenant_id: Some(tenant_id.clone()),
                        delta_cost_micros: 80,
                        delta_tokens_in: 0,
                        delta_tokens_out: 0,
                        provider_call_id: format!("call_once_{i}"),
                        updated_at_ms: 1000 + i as u64,
                    }),
                )])
                .await
                .unwrap();
        }

        let events = store.read_stream(None, 100).await.unwrap();
        let trigger_count = events
            .iter()
            .filter(|e| matches!(&e.envelope.payload, RuntimeEvent::RunCostAlertTriggered(_)))
            .count();
        assert_eq!(trigger_count, 1, "alert should fire exactly once");
    }
}
