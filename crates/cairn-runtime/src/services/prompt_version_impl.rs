use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{PromptAssetReadModel, PromptVersionReadModel, PromptVersionRecord};
use cairn_store::projections::ResourceSharingReadModel;
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
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<S> PromptVersionService for PromptVersionServiceImpl<S>
where
    S: EventLog + PromptVersionReadModel + PromptAssetReadModel + ResourceSharingReadModel + 'static,
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

        // RFC 006 deviation: the service API accepts a ProjectKey for caller
        // convenience, but RFC 006 defines prompt versions as workspace-scoped.
        // The workspace_id is extracted here and stored on the event so that
        // downstream projections can scope queries at workspace level. A future
        // revision should accept WorkspaceKey at the service boundary directly.
        let workspace_id = project.workspace_id.clone();

        // Cross-workspace access check: if the asset exists but belongs to a different
        // workspace, require a valid resource share granting "version" permission.
        if let Some(asset) = PromptAssetReadModel::get(self.store.as_ref(), &prompt_asset_id).await? {
            let asset_workspace = &asset.project.workspace_id;
            let caller_workspace = &project.workspace_id;
            if asset_workspace != caller_workspace {
                // Different workspace — check resource sharing.
                let share = ResourceSharingReadModel::get_share_for_resource(
                    self.store.as_ref(),
                    &project.tenant_id,
                    caller_workspace,
                    "prompt_asset",
                    prompt_asset_id.as_str(),
                )
                .await?;

                match share {
                    None => {
                        return Err(RuntimeError::PolicyDenied {
                            reason: format!(
                                "prompt asset '{}' belongs to workspace '{}'; no resource share grants workspace '{}' access",
                                prompt_asset_id.as_str(),
                                asset_workspace.as_str(),
                                caller_workspace.as_str(),
                            ),
                        });
                    }
                    Some(s) if !s.permissions.contains(&"version".to_owned()) => {
                        return Err(RuntimeError::PolicyDenied {
                            reason: format!(
                                "resource share for asset '{}' does not grant 'version' permission",
                                prompt_asset_id.as_str(),
                            ),
                        });
                    }
                    Some(_) => {}
                }
            }
        }

        let event = make_envelope(RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
            project: project.clone(),
            prompt_version_id: prompt_version_id.clone(),
            prompt_asset_id,
            content_hash,
            created_at: now_millis(),
            workspace_id,
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
