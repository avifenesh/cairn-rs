//! Concrete signal service implementation.
//!
//! Ingests external signals by appending a `SignalIngested` event
//! and reading back via the `SignalReadModel` projection.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::SignalReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::signals::SignalService;

pub struct SignalServiceImpl<S> {
    store: Arc<S>,
}

impl<S> SignalServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> SignalService for SignalServiceImpl<S>
where
    S: EventLog + SignalReadModel + 'static,
{
    async fn ingest(
        &self,
        project: &ProjectKey,
        signal_id: SignalId,
        source: String,
        payload: serde_json::Value,
        timestamp_ms: u64,
    ) -> Result<SignalRecord, RuntimeError> {
        let event = make_envelope(RuntimeEvent::SignalIngested(SignalIngested {
            project: project.clone(),
            signal_id: signal_id.clone(),
            source,
            payload,
            timestamp_ms,
        }));

        self.store.append(&[event]).await?;

        SignalReadModel::get(self.store.as_ref(), &signal_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("signal not found after ingest".into()))
    }

    async fn get(&self, signal_id: &SignalId) -> Result<Option<SignalRecord>, RuntimeError> {
        Ok(SignalReadModel::get(self.store.as_ref(), signal_id).await?)
    }

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SignalRecord>, RuntimeError> {
        Ok(self.store.list_by_project(project, limit, offset).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::*;
    use cairn_store::{EventLog, InMemoryStore};

    use crate::signals::SignalService;

    use super::SignalServiceImpl;

    fn test_project() -> ProjectKey {
        ProjectKey::new("tenant_acme", "ws_main", "project_alpha")
    }

    #[tokio::test]
    async fn ingest_persists_and_returns_signal() {
        let store = Arc::new(InMemoryStore::new());
        let svc = SignalServiceImpl::new(store.clone());
        let project = test_project();

        let record = svc
            .ingest(
                &project,
                SignalId::new("sig_1"),
                "webhook".to_owned(),
                serde_json::json!({"key": "value"}),
                1000,
            )
            .await
            .unwrap();

        assert_eq!(record.id, SignalId::new("sig_1"));
        assert_eq!(record.project, project);
        assert_eq!(record.source, "webhook");
        assert_eq!(record.payload, serde_json::json!({"key": "value"}));
        assert_eq!(record.timestamp_ms, 1000);
    }

    #[tokio::test]
    async fn get_returns_ingested_signal() {
        let store = Arc::new(InMemoryStore::new());
        let svc = SignalServiceImpl::new(store);
        let project = test_project();

        svc.ingest(
            &project,
            SignalId::new("sig_2"),
            "api".to_owned(),
            serde_json::json!(null),
            2000,
        )
        .await
        .unwrap();

        let found = svc.get(&SignalId::new("sig_2")).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().source, "api");
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let store = Arc::new(InMemoryStore::new());
        let svc = SignalServiceImpl::new(store);

        let result = svc.get(&SignalId::new("sig_missing")).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_by_project_returns_matching_signals() {
        let store = Arc::new(InMemoryStore::new());
        let svc = SignalServiceImpl::new(store);
        let project = test_project();
        let other_project = ProjectKey::new("tenant_other", "ws_other", "project_other");

        svc.ingest(
            &project,
            SignalId::new("sig_a"),
            "webhook".to_owned(),
            serde_json::json!(1),
            100,
        )
        .await
        .unwrap();

        svc.ingest(
            &project,
            SignalId::new("sig_b"),
            "cron".to_owned(),
            serde_json::json!(2),
            200,
        )
        .await
        .unwrap();

        svc.ingest(
            &other_project,
            SignalId::new("sig_c"),
            "other".to_owned(),
            serde_json::json!(3),
            300,
        )
        .await
        .unwrap();

        let results = svc.list_by_project(&project, 10, 0).await.unwrap();
        assert_eq!(results.len(), 2);

        let other_results = svc.list_by_project(&other_project, 10, 0).await.unwrap();
        assert_eq!(other_results.len(), 1);
        assert_eq!(other_results[0].id, SignalId::new("sig_c"));
    }

    #[tokio::test]
    async fn ingest_emits_signal_ingested_event() {
        let store = Arc::new(InMemoryStore::new());
        let svc = SignalServiceImpl::new(store.clone());
        let project = test_project();

        svc.ingest(
            &project,
            SignalId::new("sig_evt"),
            "webhook".to_owned(),
            serde_json::json!({"a": 1}),
            500,
        )
        .await
        .unwrap();

        let events = store.read_stream(None, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].envelope.payload,
            RuntimeEvent::SignalIngested(e) if e.signal_id == SignalId::new("sig_evt")
        ));
    }
}
