use async_trait::async_trait;
use cairn_domain::{ApprovalPolicyRecord, TenantId};

use crate::error::StoreError;

#[async_trait]
pub trait ApprovalPolicyReadModel: Send + Sync {
    async fn get_policy(&self, policy_id: &str)
        -> Result<Option<ApprovalPolicyRecord>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalPolicyRecord>, StoreError>;
}
