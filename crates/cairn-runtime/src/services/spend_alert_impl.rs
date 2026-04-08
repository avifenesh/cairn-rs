//! GAP-006: Spend alert service implementation.
//!
//! Tracks per-tenant thresholds in memory and checks session cost against them.
//! Emits `SpendAlertTriggered` when a session crosses the threshold (once per session).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cairn_domain::providers::{SpendAlert, SpendThresholdRecord};
use cairn_domain::{ProjectKey, RuntimeEvent};
use cairn_domain::{SessionId, SpendAlertTriggered, TenantId};
use cairn_store::projections::SessionCostReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::spend_alert::SpendAlertService;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// In-process spend alert service.
///
/// Thresholds are stored in memory (not persisted). The triggered-session set
/// prevents duplicate alerts for the same session.
pub struct SpendAlertServiceImpl<S> {
    store: Arc<S>,
    /// Per-tenant threshold in USD micros.
    thresholds: Mutex<HashMap<String, SpendThresholdRecord>>,
    /// Sessions that have already triggered an alert (dedup gate).
    triggered_sessions: Mutex<HashSet<String>>,
}

impl<S> SpendAlertServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            thresholds: Mutex::new(HashMap::new()),
            triggered_sessions: Mutex::new(HashSet::new()),
        }
    }
}

#[async_trait]
impl<S> SpendAlertService for SpendAlertServiceImpl<S>
where
    S: EventLog + SessionCostReadModel + Send + Sync + 'static,
{
    async fn set_threshold(
        &self,
        tenant_id: TenantId,
        threshold_micros: u64,
    ) -> Result<(), RuntimeError> {
        let mut thresholds = self.thresholds.lock().unwrap();
        thresholds.insert(
            tenant_id.as_str().to_owned(),
            SpendThresholdRecord {
                tenant_id,
                threshold_micros,
                set_at_ms: now_ms(),
            },
        );
        Ok(())
    }

    async fn check_session_spend(
        &self,
        session_id: &SessionId,
        tenant_id: &TenantId,
    ) -> Result<Option<SpendAlert>, RuntimeError> {
        // Look up threshold for this tenant.
        let threshold = {
            let thresholds = self.thresholds.lock().unwrap();
            thresholds
                .get(tenant_id.as_str())
                .map(|r| r.threshold_micros)
        };
        let Some(threshold_micros) = threshold else {
            return Ok(None); // No threshold configured for this tenant.
        };

        // Already triggered for this session?
        {
            let triggered = self.triggered_sessions.lock().unwrap();
            if triggered.contains(session_id.as_str()) {
                return Ok(None);
            }
        }

        // Read session cost from store.
        let cost = SessionCostReadModel::get_session_cost(self.store.as_ref(), session_id).await?;
        let total = cost.map(|c| c.total_cost_micros).unwrap_or(0);

        if total < threshold_micros {
            return Ok(None);
        }

        // Threshold exceeded — emit event and mark session as triggered.
        let alert_id = format!("alert_{}_{}", tenant_id.as_str(), now_ms());
        let sentinel = ProjectKey::new(tenant_id.as_str(), "system", "system");

        let event = make_envelope(RuntimeEvent::SpendAlertTriggered(SpendAlertTriggered {
            project: sentinel,
            alert_id: alert_id.clone(),
            tenant_id: tenant_id.clone(),
            session_id: session_id.clone(),
            threshold_micros,
            current_micros: total,
            triggered_at_ms: now_ms(),
        }));
        self.store.append(&[event]).await?;

        self.triggered_sessions
            .lock()
            .unwrap()
            .insert(session_id.as_str().to_owned());

        Ok(Some(SpendAlert {
            alert_id,
            tenant_id: tenant_id.clone(),
            threshold_micros,
            current_micros: total,
            triggered_at_ms: now_ms(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{EventEnvelope, EventId, EventSource, ProjectKey, SessionCostUpdated};
    use cairn_store::InMemoryStore;

    async fn accumulate_cost(
        store: &Arc<InMemoryStore>,
        session_id: &SessionId,
        tenant_id: &TenantId,
        cost_micros: u64,
    ) {
        store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new(format!("evt_cost_{cost_micros}")),
                EventSource::Runtime,
                RuntimeEvent::SessionCostUpdated(SessionCostUpdated {
                    project: ProjectKey::new(tenant_id.as_str(), "w", "p"),
                    session_id: session_id.clone(),
                    tenant_id: tenant_id.clone(),
                    delta_cost_micros: cost_micros,
                    delta_tokens_in: 100,
                    delta_tokens_out: 50,
                    provider_call_id: format!("call_{cost_micros}"),
                    updated_at_ms: 1000,
                }),
            )])
            .await
            .unwrap();
    }

    /// Manager spec: set threshold=1000, accumulate 1200, assert SpendAlertTriggered.
    #[tokio::test]
    async fn spend_alert_triggered_when_session_exceeds_threshold() {
        let store = Arc::new(InMemoryStore::new());
        let svc = SpendAlertServiceImpl::new(store.clone());
        let tenant_id = TenantId::new("tenant_spend");
        let session_id = SessionId::new("sess_spend_1");

        // Set threshold at 1000 µUSD.
        svc.set_threshold(tenant_id.clone(), 1_000).await.unwrap();

        // Accumulate 1200 µUSD across two calls (600 + 600).
        accumulate_cost(&store, &session_id, &tenant_id, 600).await;
        accumulate_cost(&store, &session_id, &tenant_id, 600).await;

        // Check: should fire.
        let alert = svc
            .check_session_spend(&session_id, &tenant_id)
            .await
            .unwrap();

        assert!(
            alert.is_some(),
            "alert must fire when cost (1200) > threshold (1000)"
        );
        let alert = alert.unwrap();
        assert_eq!(alert.threshold_micros, 1_000);
        assert_eq!(alert.current_micros, 1_200);
        assert!(alert.triggered_at_ms > 0);

        // Verify SpendAlertTriggered event was appended.
        let events = store.read_stream(None, 50).await.unwrap();
        let triggered = events.iter().any(|e| {
            matches!(
                &e.envelope.payload,
                RuntimeEvent::SpendAlertTriggered(ev)
                    if ev.tenant_id == tenant_id
                        && ev.threshold_micros == 1_000
                        && ev.current_micros == 1_200
            )
        });
        assert!(
            triggered,
            "SpendAlertTriggered event must be in the event log"
        );
    }

    /// Alert must NOT fire when cost is below threshold.
    #[tokio::test]
    async fn spend_alert_not_triggered_below_threshold() {
        let store = Arc::new(InMemoryStore::new());
        let svc = SpendAlertServiceImpl::new(store.clone());
        let tenant_id = TenantId::new("tenant_low");
        let session_id = SessionId::new("sess_low");

        svc.set_threshold(tenant_id.clone(), 2_000).await.unwrap();
        accumulate_cost(&store, &session_id, &tenant_id, 500).await;

        let alert = svc
            .check_session_spend(&session_id, &tenant_id)
            .await
            .unwrap();
        assert!(
            alert.is_none(),
            "alert must not fire when cost (500) < threshold (2000)"
        );

        let events = store.read_stream(None, 50).await.unwrap();
        let triggered = events
            .iter()
            .any(|e| matches!(&e.envelope.payload, RuntimeEvent::SpendAlertTriggered(_)));
        assert!(!triggered, "no SpendAlertTriggered event should be in log");
    }

    /// Alert fires at most once per session (dedup gate).
    #[tokio::test]
    async fn spend_alert_fires_only_once_per_session() {
        let store = Arc::new(InMemoryStore::new());
        let svc = SpendAlertServiceImpl::new(store.clone());
        let tenant_id = TenantId::new("tenant_once");
        let session_id = SessionId::new("sess_once");

        svc.set_threshold(tenant_id.clone(), 500).await.unwrap();
        accumulate_cost(&store, &session_id, &tenant_id, 600).await;

        // First check — fires.
        let first = svc
            .check_session_spend(&session_id, &tenant_id)
            .await
            .unwrap();
        assert!(first.is_some(), "first check must fire");

        // Second check — same session, should NOT fire again.
        let second = svc
            .check_session_spend(&session_id, &tenant_id)
            .await
            .unwrap();
        assert!(
            second.is_none(),
            "second check on same session must not fire again"
        );

        let events = store.read_stream(None, 50).await.unwrap();
        let trigger_count = events
            .iter()
            .filter(|e| matches!(&e.envelope.payload, RuntimeEvent::SpendAlertTriggered(_)))
            .count();
        assert_eq!(
            trigger_count, 1,
            "exactly one SpendAlertTriggered per session"
        );
    }

    /// No threshold configured → no alert.
    #[tokio::test]
    async fn spend_alert_requires_threshold_to_be_set() {
        let store = Arc::new(InMemoryStore::new());
        let svc = SpendAlertServiceImpl::new(store.clone());
        let tenant_id = TenantId::new("tenant_none");
        let session_id = SessionId::new("sess_none");

        // No set_threshold call — check should return None immediately.
        accumulate_cost(&store, &session_id, &tenant_id, 9_999_999).await;
        let alert = svc
            .check_session_spend(&session_id, &tenant_id)
            .await
            .unwrap();
        assert!(alert.is_none(), "no alert when no threshold is configured");
    }

    /// Multiple tenants have independent thresholds.
    #[tokio::test]
    async fn spend_alert_independent_per_tenant() {
        let store = Arc::new(InMemoryStore::new());
        let svc = SpendAlertServiceImpl::new(store.clone());
        let tenant_a = TenantId::new("tenant_a");
        let tenant_b = TenantId::new("tenant_b");
        let sess_a = SessionId::new("sess_a");
        let sess_b = SessionId::new("sess_b");

        svc.set_threshold(tenant_a.clone(), 1_000).await.unwrap();
        svc.set_threshold(tenant_b.clone(), 5_000).await.unwrap();

        accumulate_cost(&store, &sess_a, &tenant_a, 1_200).await;
        accumulate_cost(&store, &sess_b, &tenant_b, 1_200).await;

        let alert_a = svc.check_session_spend(&sess_a, &tenant_a).await.unwrap();
        let alert_b = svc.check_session_spend(&sess_b, &tenant_b).await.unwrap();

        assert!(
            alert_a.is_some(),
            "tenant_a threshold (1000) exceeded by 1200"
        );
        assert!(
            alert_b.is_none(),
            "tenant_b threshold (5000) not exceeded by 1200"
        );
    }
}
