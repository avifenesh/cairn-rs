use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{PromptVersionReadModel, PromptVersionRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::prompt_versions::PromptVersionService;

pub struct PromptVersionServiceImpl<S> {
    store: Arc<S>,
}

impl<S> PromptVersionServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[async_trait]
impl<S> PromptVersionService for PromptVersionServiceImpl<S>
where
    S: EventLog + PromptVersionReadModel + 'static,
{
    async fn create(
        &self,
        project: &ProjectKey,
        prompt_version_id: PromptVersionId,
        prompt_asset_id: PromptAssetId,
        content_hash: String,
    ) -> Result<PromptVersionRecord, RuntimeError> {
        if PromptVersionReadModel::get(self.store.as_ref(), &prompt_version_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "prompt_version",
                id: prompt_version_id.to_string(),
            });
        }

        let event = make_envelope(RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
            project: project.clone(),
            prompt_version_id: prompt_version_id.clone(),
            prompt_asset_id,
            content_hash,
            created_at: now_millis(),
        }));

        self.store.append(&[event]).await?;

        PromptVersionReadModel::get(self.store.as_ref(), &prompt_version_id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("prompt_version not found after create".into())
            })
    }

    async fn get(
        &self,
        prompt_version_id: &PromptVersionId,
    ) -> Result<Option<PromptVersionRecord>, RuntimeError> {
        Ok(PromptVersionReadModel::get(self.store.as_ref(), prompt_version_id).await?)
    }

    async fn list_by_asset(
        &self,
        prompt_asset_id: &PromptAssetId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<PromptVersionRecord>, RuntimeError> {
        Ok(self
            .store
            .list_by_asset(prompt_asset_id, limit, offset)
            .await?)
    }
}
