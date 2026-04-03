use async_trait::async_trait;
use cairn_domain::{CheckpointDisposition, CheckpointId, ProjectKey, RunId};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Current-state record for a checkpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub checkpoint_id: CheckpointId,
    pub project: ProjectKey,
    pub run_id: RunId,
    pub disposition: CheckpointDisposition,
    pub version: u64,
    pub created_at: u64,
}

/// Read-model for checkpoint current state.
#[async_trait]
pub trait CheckpointReadModel: Send + Sync {
    async fn get(
        &self,
        checkpoint_id: &CheckpointId,
    ) -> Result<Option<CheckpointRecord>, StoreError>;

    /// Get the latest checkpoint for a run (used by recovery).
    async fn latest_for_run(&self, run_id: &RunId) -> Result<Option<CheckpointRecord>, StoreError>;

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<CheckpointRecord>, StoreError>;
}
