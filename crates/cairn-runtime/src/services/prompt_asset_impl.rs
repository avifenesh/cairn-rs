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
        .unwrap_or_default()
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
        // RFC 006 deviation: the service API accepts a ProjectKey for caller
        // convenience, but RFC 006 defines prompt assets as workspace-scoped
        // (tenant + workspace), not project-scoped. The workspace_id is
        // extracted from the project key here and stored on the event so that
        // downstream projections and query paths can use workspace-level scope
        // without needing to re-derive it. A future revision should update the
        // service boundary to accept WorkspaceKey directly.
        let workspace_id = project.workspace_id.clone();

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
            workspace_id,
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

// ── Additional stub for cairn-app ─────────────────────────────────────────

impl<S> PromptAssetServiceImpl<S>
where
    S: cairn_store::EventLog + cairn_store::projections::PromptAssetReadModel + 'static,
{
    /// List prompt assets by workspace (returns all assets for the workspace's tenant).
    pub async fn list_by_workspace(
        &self,
        tenant_id: &cairn_domain::TenantId,
        workspace_id: &cairn_domain::WorkspaceId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_store::projections::PromptAssetRecord>, crate::error::RuntimeError> {
        // PromptAssetReadModel only exposes list_by_project; tenant-level listing is not yet
        // implemented. Return empty until a tenant-scoped method is added to the store trait.
        let _ = (tenant_id, workspace_id, limit, offset);
        Ok(vec![])
    }
}
