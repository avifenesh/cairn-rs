use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{CheckpointReadModel, CheckpointRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::checkpoints::CheckpointService;
use crate::error::RuntimeError;

pub struct CheckpointServiceImpl<S> {
    store: Arc<S>,
}

impl<S> CheckpointServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> CheckpointService for CheckpointServiceImpl<S>
where
    S: EventLog + CheckpointReadModel + 'static,
{
    async fn save(
        &self,
        project: &ProjectKey,
        run_id: &RunId,
        checkpoint_id: CheckpointId,
    ) -> Result<CheckpointRecord, RuntimeError> {
        let event = make_envelope(RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
            project: project.clone(),
            run_id: run_id.clone(),
            checkpoint_id: checkpoint_id.clone(),
            disposition: CheckpointDisposition::Latest,
            data: None,
        }));

        self.store.append(&[event]).await?;

        CheckpointReadModel::get(self.store.as_ref(), &checkpoint_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("checkpoint not found after save".into()))
    }

    async fn get(
        &self,
        checkpoint_id: &CheckpointId,
    ) -> Result<Option<CheckpointRecord>, RuntimeError> {
        Ok(CheckpointReadModel::get(self.store.as_ref(), checkpoint_id).await?)
    }

    async fn latest_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<CheckpointRecord>, RuntimeError> {
        Ok(self.store.latest_for_run(run_id).await?)
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<CheckpointRecord>, RuntimeError> {
        Ok(self.store.list_by_run(run_id, limit).await?)
    }
}
