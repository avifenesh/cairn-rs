use async_trait::async_trait;
use cairn_domain::policy::GuardrailPolicy;
use cairn_domain::TenantId;

use crate::error::StoreError;

#[async_trait]
pub trait GuardrailReadModel: Send + Sync {
    async fn get_policy(&self, policy_id: &str) -> Result<Option<GuardrailPolicy>, StoreError>;

    async fn list_policies(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<GuardrailPolicy>, StoreError>;
}
