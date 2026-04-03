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
