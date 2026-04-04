use async_trait::async_trait;
use cairn_domain::workers::ExternalWorkerRecord;
use cairn_domain::{TenantId, WorkerId};

use crate::error::StoreError;

#[async_trait]
pub trait ExternalWorkerReadModel: Send + Sync {
    async fn get(&self, id: &WorkerId) -> Result<Option<ExternalWorkerRecord>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ExternalWorkerRecord>, StoreError>;
}
