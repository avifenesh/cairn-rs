use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{ApprovalReadModel, ApprovalRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::approvals::ApprovalService;
use crate::error::RuntimeError;

pub struct ApprovalServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ApprovalServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> ApprovalService for ApprovalServiceImpl<S>
where
    S: EventLog + ApprovalReadModel + 'static,
{
    async fn request(
        &self,
        project: &ProjectKey,
        approval_id: ApprovalId,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
        requirement: ApprovalRequirement,
    ) -> Result<ApprovalRecord, RuntimeError> {
        let event = make_envelope(RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: project.clone(),
            approval_id: approval_id.clone(),
            run_id,
            task_id,
            requirement,
        }));

        self.store.append(&[event]).await?;

        ApprovalReadModel::get(self.store.as_ref(), &approval_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("approval not found after request".into()))
    }

    async fn get(&self, approval_id: &ApprovalId) -> Result<Option<ApprovalRecord>, RuntimeError> {
        Ok(ApprovalReadModel::get(self.store.as_ref(), approval_id).await?)
    }

    async fn resolve(
        &self,
        approval_id: &ApprovalId,
        decision: ApprovalDecision,
    ) -> Result<ApprovalRecord, RuntimeError> {
        let approval = ApprovalReadModel::get(self.store.as_ref(), approval_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "approval",
                id: approval_id.to_string(),
            })?;

        if approval.decision.is_some() {
            return Err(RuntimeError::InvalidTransition {
                entity: "approval",
                from: "resolved".into(),
                to: format!("{decision:?}"),
            });
        }

        let event = make_envelope(RuntimeEvent::ApprovalResolved(ApprovalResolved {
            project: approval.project.clone(),
            approval_id: approval_id.clone(),
            decision,
        }));

        self.store.append(&[event]).await?;

        ApprovalReadModel::get(self.store.as_ref(), approval_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("approval not found after resolve".into()))
    }

    async fn list_pending(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, RuntimeError> {
        Ok(self.store.list_pending(project, limit, offset).await?)
    }
}
