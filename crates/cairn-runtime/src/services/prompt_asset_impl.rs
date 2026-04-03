use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{PromptAssetReadModel, PromptAssetRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::prompt_assets::PromptAssetService;

pub struct PromptAssetServiceImpl<S> {
    store: Arc<S>,
}

impl<S> PromptAssetServiceImpl<S> {
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
impl<S> PromptAssetService for PromptAssetServiceImpl<S>
where
    S: EventLog + PromptAssetReadModel + 'static,
{
    async fn create(
        &self,
        project: &ProjectKey,
        prompt_asset_id: PromptAssetId,
        name: String,
        kind: String,
    ) -> Result<PromptAssetRecord, RuntimeError> {
        if PromptAssetReadModel::get(self.store.as_ref(), &prompt_asset_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "prompt_asset",
                id: prompt_asset_id.to_string(),
            });
        }

        let event = make_envelope(RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
            project: project.clone(),
            prompt_asset_id: prompt_asset_id.clone(),
            name,
            kind,
            created_at: now_millis(),
        }));

        self.store.append(&[event]).await?;

        PromptAssetReadModel::get(self.store.as_ref(), &prompt_asset_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("prompt_asset not found after create".into()))
    }

    async fn get(
        &self,
        prompt_asset_id: &PromptAssetId,
    ) -> Result<Option<PromptAssetRecord>, RuntimeError> {
        Ok(PromptAssetReadModel::get(self.store.as_ref(), prompt_asset_id).await?)
    }

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<PromptAssetRecord>, RuntimeError> {
        Ok(self.store.list_by_project(project, limit, offset).await?)
    }
}
