use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{PromptReleaseReadModel, PromptReleaseRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::prompt_releases::PromptReleaseService;

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

pub struct PromptReleaseServiceImpl<S> {
    store: Arc<S>,
}

impl<S> PromptReleaseServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> PromptReleaseService for PromptReleaseServiceImpl<S>
where
    S: EventLog + PromptReleaseReadModel + 'static,
{
    async fn create(
        &self,
        project: &ProjectKey,
        release_id: PromptReleaseId,
        asset_id: PromptAssetId,
        version_id: PromptVersionId,
    ) -> Result<PromptReleaseRecord, RuntimeError> {
        if PromptReleaseReadModel::get(self.store.as_ref(), &release_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "prompt_release",
                id: release_id.to_string(),
            });
        }

        let event = make_envelope(RuntimeEvent::PromptReleaseCreated(PromptReleaseCreated {
            project: project.clone(),
            prompt_release_id: release_id.clone(),
            prompt_asset_id: asset_id,
            prompt_version_id: version_id,
            created_at: now_millis(),
        }));

        self.store.append(&[event]).await?;

        PromptReleaseReadModel::get(self.store.as_ref(), &release_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("prompt_release not found after create".into()))
    }

    async fn transition(
        &self,
        release_id: &PromptReleaseId,
        to_state: &str,
    ) -> Result<PromptReleaseRecord, RuntimeError> {
        let existing = PromptReleaseReadModel::get(self.store.as_ref(), release_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "prompt_release",
                id: release_id.to_string(),
            })?;

        let event = make_envelope(RuntimeEvent::PromptReleaseTransitioned(
            PromptReleaseTransitioned {
                project: existing.project.clone(),
                prompt_release_id: release_id.clone(),
                from_state: existing.state.clone(),
                to_state: to_state.to_owned(),
                transitioned_at: now_millis(),
            },
        ));

        self.store.append(&[event]).await?;

        PromptReleaseReadModel::get(self.store.as_ref(), release_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("prompt_release not found after transition".into()))
    }

    async fn activate(
        &self,
        release_id: &PromptReleaseId,
    ) -> Result<PromptReleaseRecord, RuntimeError> {
        let existing = PromptReleaseReadModel::get(self.store.as_ref(), release_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "prompt_release",
                id: release_id.to_string(),
            })?;

        // Deactivate any currently active release for this asset.
        let all = self
            .store
            .list_by_project(&existing.project, 1000, 0)
            .await?;
        for rel in &all {
            if rel.prompt_asset_id == existing.prompt_asset_id
                && rel.state == "active"
                && rel.prompt_release_id != *release_id
            {
                let deactivate = make_envelope(RuntimeEvent::PromptReleaseTransitioned(
                    PromptReleaseTransitioned {
                        project: existing.project.clone(),
                        prompt_release_id: rel.prompt_release_id.clone(),
                        from_state: "active".to_owned(),
                        to_state: "archived".to_owned(),
                        transitioned_at: now_millis(),
                    },
                ));
                self.store.append(&[deactivate]).await?;
            }
        }

        // Activate the target release.
        self.transition(release_id, "active").await
    }

    async fn rollback(
        &self,
        current_id: &PromptReleaseId,
        target_id: &PromptReleaseId,
    ) -> Result<PromptReleaseRecord, RuntimeError> {
        // Deactivate current.
        self.transition(current_id, "archived").await?;
        // Reactivate target.
        self.transition(target_id, "active").await
    }

    async fn resolve(
        &self,
        project: &ProjectKey,
        asset_id: &PromptAssetId,
        selector: &str,
    ) -> Result<Option<PromptReleaseRecord>, RuntimeError> {
        Ok(self
            .store
            .active_for_selector(project, asset_id, selector)
            .await?)
    }

    async fn get(
        &self,
        release_id: &PromptReleaseId,
    ) -> Result<Option<PromptReleaseRecord>, RuntimeError> {
        Ok(PromptReleaseReadModel::get(self.store.as_ref(), release_id).await?)
    }

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<PromptReleaseRecord>, RuntimeError> {
        Ok(self.store.list_by_project(project, limit, offset).await?)
    }
}
