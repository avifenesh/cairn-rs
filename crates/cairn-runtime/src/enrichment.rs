//! Store-backed enrichment seam for API/SSE surfaces.
//!
//! Worker 8 uses this to enrich SSE frames and API responses with
//! product-level data (title, description, progress, context) from
//! the store's read models. This is the single stable seam — Worker 8
//! should not query store projections directly.

use async_trait::async_trait;
use cairn_domain::{
    ApprovalDecision, ApprovalId, ApprovalRequirement, CheckpointDisposition, CheckpointId,
    ProjectKey, RunId, RunState, SessionId, SessionState, TaskId, TaskState,
};
use serde::{Deserialize, Serialize};

use crate::error::RuntimeError;

/// Enriched task data for SSE/API surfaces.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskEnrichment {
    pub task_id: TaskId,
    pub project: ProjectKey,
    pub state: TaskState,
    pub title: Option<String>,
    pub description: Option<String>,
    pub parent_run_id: Option<RunId>,
    pub lease_owner: Option<String>,
}

/// Enriched approval data for SSE/API surfaces.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalEnrichment {
    pub approval_id: ApprovalId,
    pub project: ProjectKey,
    pub requirement: ApprovalRequirement,
    pub decision: Option<ApprovalDecision>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
}

/// Enriched run data for SSE/API surfaces.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunEnrichment {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub project: ProjectKey,
    pub state: RunState,
    pub parent_run_id: Option<RunId>,
}

/// Enriched session data for SSE/API surfaces.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionEnrichment {
    pub session_id: SessionId,
    pub project: ProjectKey,
    pub state: SessionState,
}

/// Enriched checkpoint data for SSE/API surfaces.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckpointEnrichment {
    pub checkpoint_id: CheckpointId,
    pub run_id: RunId,
    pub disposition: CheckpointDisposition,
}

/// Store-backed enrichment for SSE/API surfaces.
///
/// **Contract for Worker 8 (API/SSE):**
/// - Depend on `RuntimeEnrichment` trait, not on store projections directly.
/// - Each `enrich_*` method returns `None` if the entity doesn't exist (not an error).
/// - Enrichment structs contain the product-level fields needed for SSE frame shaping
///   (title, description, state, IDs). They are `Serialize` for direct JSON embedding.
/// - This seam is the only stable path for store-backed SSE/API enrichment.
///   Do not query `cairn_store::projections::*ReadModel` from the API layer.
#[async_trait]
pub trait RuntimeEnrichment: Send + Sync {
    async fn enrich_task(&self, task_id: &TaskId) -> Result<Option<TaskEnrichment>, RuntimeError>;
    async fn enrich_approval(
        &self,
        approval_id: &ApprovalId,
    ) -> Result<Option<ApprovalEnrichment>, RuntimeError>;
    async fn enrich_run(&self, run_id: &RunId) -> Result<Option<RunEnrichment>, RuntimeError>;
    async fn enrich_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionEnrichment>, RuntimeError>;
    async fn enrich_checkpoint(
        &self,
        checkpoint_id: &CheckpointId,
    ) -> Result<Option<CheckpointEnrichment>, RuntimeError>;
}

/// Concrete implementation backed by store read models.
pub struct StoreBackedEnrichment<S> {
    store: std::sync::Arc<S>,
}

impl<S> StoreBackedEnrichment<S> {
    pub fn new(store: std::sync::Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> RuntimeEnrichment for StoreBackedEnrichment<S>
where
    S: cairn_store::projections::TaskReadModel
        + cairn_store::projections::ApprovalReadModel
        + cairn_store::projections::RunReadModel
        + cairn_store::projections::SessionReadModel
        + cairn_store::projections::CheckpointReadModel
        + 'static,
{
    async fn enrich_task(&self, task_id: &TaskId) -> Result<Option<TaskEnrichment>, RuntimeError> {
        let rec =
            cairn_store::projections::TaskReadModel::get(self.store.as_ref(), task_id).await?;
        Ok(rec.map(|r| TaskEnrichment {
            task_id: r.task_id,
            project: r.project,
            state: r.state,
            title: r.title,
            description: r.description,
            parent_run_id: r.parent_run_id,
            lease_owner: r.lease_owner,
        }))
    }

    async fn enrich_approval(
        &self,
        approval_id: &ApprovalId,
    ) -> Result<Option<ApprovalEnrichment>, RuntimeError> {
        let rec =
            cairn_store::projections::ApprovalReadModel::get(self.store.as_ref(), approval_id)
                .await?;
        Ok(rec.map(|r| ApprovalEnrichment {
            approval_id: r.approval_id,
            project: r.project,
            requirement: r.requirement,
            decision: r.decision,
            title: r.title,
            description: r.description,
            run_id: r.run_id,
            task_id: r.task_id,
        }))
    }

    async fn enrich_run(&self, run_id: &RunId) -> Result<Option<RunEnrichment>, RuntimeError> {
        let rec = cairn_store::projections::RunReadModel::get(self.store.as_ref(), run_id).await?;
        Ok(rec.map(|r| RunEnrichment {
            run_id: r.run_id,
            session_id: r.session_id,
            project: r.project,
            state: r.state,
            parent_run_id: r.parent_run_id,
        }))
    }

    async fn enrich_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionEnrichment>, RuntimeError> {
        let rec = cairn_store::projections::SessionReadModel::get(self.store.as_ref(), session_id)
            .await?;
        Ok(rec.map(|r| SessionEnrichment {
            session_id: r.session_id,
            project: r.project,
            state: r.state,
        }))
    }

    async fn enrich_checkpoint(
        &self,
        checkpoint_id: &CheckpointId,
    ) -> Result<Option<CheckpointEnrichment>, RuntimeError> {
        let rec =
            cairn_store::projections::CheckpointReadModel::get(self.store.as_ref(), checkpoint_id)
                .await?;
        Ok(rec.map(|r| CheckpointEnrichment {
            checkpoint_id: r.checkpoint_id,
            run_id: r.run_id,
            disposition: r.disposition,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrichment_types_are_serializable() {
        let task = TaskEnrichment {
            task_id: TaskId::new("t1"),
            project: ProjectKey::new("t", "w", "p"),
            state: TaskState::Running,
            title: Some("Review PR".to_owned()),
            description: Some("Review the pull request".to_owned()),
            parent_run_id: Some(RunId::new("r1")),
            lease_owner: Some("worker-a".to_owned()),
        };
        let json = serde_json::to_string(&task).unwrap();
        assert!(json.contains("Review PR"));
    }
}
