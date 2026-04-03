use async_trait::async_trait;
use cairn_domain::{ProjectKey, SignalId, SignalRecord};

use crate::error::StoreError;

/// Read-model for signal current state.
#[async_trait]
pub trait SignalReadModel: Send + Sync {
    async fn get(&self, signal_id: &SignalId) -> Result<Option<SignalRecord>, StoreError>;

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SignalRecord>, StoreError>;
}
