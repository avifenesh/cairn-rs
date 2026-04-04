use async_trait::async_trait;
use cairn_domain::{EntitlementOverrideRecord, LicenseRecord, TenantId};

use crate::error::StoreError;

#[async_trait]
pub trait LicenseReadModel: Send + Sync {
    async fn get_active(&self, tenant_id: &TenantId) -> Result<Option<LicenseRecord>, StoreError>;

    async fn list_overrides(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<EntitlementOverrideRecord>, StoreError>;
}
