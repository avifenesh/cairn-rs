use async_trait::async_trait;
use cairn_domain::{TenantId, TenantQuota};

use crate::error::StoreError;

#[async_trait]
pub trait QuotaReadModel: Send + Sync {
    async fn get_quota(&self, tenant_id: &TenantId) -> Result<Option<TenantQuota>, StoreError>;
}
