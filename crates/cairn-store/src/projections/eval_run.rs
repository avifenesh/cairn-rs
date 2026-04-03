use async_trait::async_trait;
use cairn_domain::{EvalRunId, ProjectKey};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Current-state record for an eval run projection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvalRunRecord {
    pub eval_run_id: EvalRunId,
    pub project: ProjectKey,
    pub subject_kind: String,
    pub evaluator_type: String,
    pub success: Option<bool>,
    pub error_message: Option<String>,
    pub started_at: u64,
    pub completed_at: Option<u64>,
}

/// Read-model for eval run current state.
#[async_trait]
pub trait EvalRunReadModel: Send + Sync {
    async fn get(&self, eval_run_id: &EvalRunId) -> Result<Option<EvalRunRecord>, StoreError>;

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<EvalRunRecord>, StoreError>;
}
