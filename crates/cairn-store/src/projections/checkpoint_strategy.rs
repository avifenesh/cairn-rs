use async_trait::async_trait;
use cairn_domain::{CheckpointStrategy, RunId};

use crate::error::StoreError;

#[async_trait]
pub trait CheckpointStrategyReadModel: Send + Sync {
    async fn get_by_run(&self, run_id: &RunId) -> Result<Option<CheckpointStrategy>, StoreError>;
}
