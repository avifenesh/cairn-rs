use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{
    ApprovalReadModel, ApprovalRecord, PromptReleaseReadModel, PromptReleaseRecord,
};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::prompt_releases::PromptReleaseService;

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub struct PromptReleaseServiceImpl<S> {
    store: Arc<S>,
    /// RFC 006: maps release_id → policy_id for attached approval policies.
    policy_attachments: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// RFC 006: maps release_id → approval_id for requested approvals.
    approval_links: std::sync::Mutex<std::collections::HashMap<String, cairn_domain::ApprovalId>>,
}

impl<S> PromptReleaseServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            policy_attachments: std::sync::Mutex::new(std::collections::HashMap::new()),
            approval_links: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl<S> PromptReleaseService for PromptReleaseServiceImpl<S>
where
    S: EventLog + PromptReleaseReadModel + ApprovalReadModel + 'static,
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
            release_tag: None,
            created_by: None,
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
                actor: None,
                reason: None,
            },
        ));

        self.store.append(&[event]).await?;

        PromptReleaseReadModel::get(self.store.as_ref(), release_id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("prompt_release not found after transition".into())
            })
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

        // RFC 006: activation is only valid from "approved" state.
        // V1 must not support silent auto-promotion from draft to active.
        if existing.state != "approved" {
            return Err(RuntimeError::InvalidTransition {
                entity: "prompt_release",
                from: existing.state.clone(),
                to: "active".to_owned(),
            });
        }

        // RFC 006: if an approval policy is attached, check approval status.
        let policy_id = self
            .policy_attachments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(release_id.as_str())
            .cloned();
        if let Some(_pid) = policy_id {
            let approval_id = self
                .approval_links
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(release_id.as_str())
                .cloned();
            match approval_id {
                None => {
                    return Err(RuntimeError::PolicyDenied {
                        reason: format!(
                            "release {} requires approval before activation — call request_approval first",
                            release_id.as_str()
                        ),
                    });
                }
                Some(aid) => {
                    let approval = ApprovalReadModel::get(self.store.as_ref(), &aid)
                        .await?
                        .ok_or_else(|| RuntimeError::Internal("approval record missing".into()))?;
                    match approval.decision {
                        Some(cairn_domain::ApprovalDecision::Approved) => {}
                        Some(cairn_domain::ApprovalDecision::Rejected) => {
                            return Err(RuntimeError::PolicyDenied {
                                reason: "approval was rejected; create a new release to retry"
                                    .to_owned(),
                            });
                        }
                        None => {
                            return Err(RuntimeError::PolicyDenied {
                                reason:
                                    "approval is pending; wait for resolution before activating"
                                        .to_owned(),
                            });
                        }
                    }
                }
            }
        }

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
                        to_state: "approved".to_owned(),
                        transitioned_at: now_millis(),
                        actor: None,
                        reason: Some("superseded by newer active release".to_owned()),
                    },
                ));
                self.store.append(&[deactivate]).await?;
            }
        }

        // Activate the target release.
        self.transition(release_id, "active").await
    }

    async fn attach_approval_policy(
        &self,
        release_id: &PromptReleaseId,
        policy_id: &str,
    ) -> Result<(), RuntimeError> {
        // Verify the release exists.
        PromptReleaseReadModel::get(self.store.as_ref(), release_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "prompt_release",
                id: release_id.to_string(),
            })?;
        // Store policy attachment.
        self.policy_attachments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(release_id.as_str().to_owned(), policy_id.to_owned());
        Ok(())
    }

    async fn request_approval(
        &self,
        release_id: &PromptReleaseId,
    ) -> Result<ApprovalRecord, RuntimeError> {
        use cairn_domain::{ApprovalId, ApprovalRequirement};
        let existing = PromptReleaseReadModel::get(self.store.as_ref(), release_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "prompt_release",
                id: release_id.to_string(),
            })?;
        let approval_id = ApprovalId::new(format!("apr_rel_{}", release_id.as_str()));
        let event = make_envelope(RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: existing.project.clone(),
            approval_id: approval_id.clone(),
            run_id: None,
            task_id: None,
            requirement: ApprovalRequirement::Required,
            title: Some(format!("Approve prompt release: {}", release_id.as_str())),
            description: Some(format!(
                "Prompt release `{}` requires operator approval before activation.",
                release_id.as_str()
            )),
        }));
        self.store.append(&[event]).await?;
        // Store the release → approval link.
        self.approval_links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(release_id.as_str().to_owned(), approval_id.clone());
        ApprovalReadModel::get(self.store.as_ref(), &approval_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("approval not found after request".into()))
    }

    async fn start_rollout(
        &self,
        release_id: &PromptReleaseId,
        percent: u8,
    ) -> Result<PromptReleaseRecord, RuntimeError> {
        let existing = PromptReleaseReadModel::get(self.store.as_ref(), release_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "prompt_release",
                id: release_id.to_string(),
            })?;
        let event = make_envelope(RuntimeEvent::PromptRolloutStarted(PromptRolloutStarted {
            project: existing.project.clone(),
            prompt_release_id: release_id.clone(),
            percent,
            started_at: now_millis(),
            release_id: Some(release_id.clone()),
        }));
        self.store.append(&[event]).await?;
        PromptReleaseReadModel::get(self.store.as_ref(), release_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("not found after rollout".into()))
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
