use async_trait::async_trait;
use cairn_domain::{ProjectKey, RunId};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PauseScheduledRecord {
    pub run_id: RunId,
    pub project: ProjectKey,
    pub resume_at_ms: u64,
    pub created_at_ms: u64,
}

#[async_trait]
pub trait PauseScheduleReadModel: Send + Sync {
    async fn list_due(&self, before_ms: u64) -> Result<Vec<PauseScheduledRecord>, StoreError>;
}
