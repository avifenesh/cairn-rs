use async_trait::async_trait;
use cairn_domain::{
    EntitlementOverrideRecord, FeatureGateResult, LicenseRecord, ProductTier, TenantId,
};

use crate::error::RuntimeError;

#[async_trait]
pub trait LicenseService: Send + Sync {
    async fn activate(
        &self,
        tenant_id: TenantId,
        tier: ProductTier,
        valid_until_ms: Option<u64>,
    ) -> Result<LicenseRecord, RuntimeError>;

    async fn get_active(&self, tenant_id: &TenantId)
        -> Result<Option<LicenseRecord>, RuntimeError>;

    async fn set_override(
        &self,
        tenant_id: TenantId,
        feature: String,
        allowed: bool,
        reason: Option<String>,
    ) -> Result<EntitlementOverrideRecord, RuntimeError>;

    async fn check_feature(
        &self,
        tenant_id: &TenantId,
        feature: &str,
    ) -> Result<FeatureGateResult, RuntimeError>;
}
