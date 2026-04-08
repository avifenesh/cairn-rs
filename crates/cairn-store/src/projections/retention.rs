use async_trait::async_trait;
use cairn_domain::{RetentionPolicy, RetentionResult, TenantId};

use crate::error::StoreError;

#[async_trait]
pub trait RetentionPolicyReadModel: Send + Sync {
    async fn get_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Option<RetentionPolicy>, StoreError>;
}

#[async_trait]
pub trait RetentionMaintenance: Send + Sync {
    async fn apply_retention(&self, tenant_id: &TenantId) -> Result<RetentionResult, StoreError>;
}
