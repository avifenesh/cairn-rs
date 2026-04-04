use async_trait::async_trait;
use cairn_domain::{EvalDataset, TenantId};

use crate::error::StoreError;

#[async_trait]
pub trait EvalDatasetReadModel: Send + Sync {
    async fn get_dataset(&self, dataset_id: &str) -> Result<Option<EvalDataset>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<EvalDataset>, StoreError>;
}
