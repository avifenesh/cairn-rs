use async_trait::async_trait;
use cairn_domain::credentials::{CredentialRecord, CredentialRotationRecord};
use cairn_domain::{CredentialId, TenantId};

use crate::error::StoreError;

#[async_trait]
pub trait CredentialReadModel: Send + Sync {
    async fn get(&self, id: &CredentialId) -> Result<Option<CredentialRecord>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<CredentialRecord>, StoreError>;
}

#[async_trait]
pub trait CredentialRotationReadModel: Send + Sync {
    async fn list_rotations(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<CredentialRotationRecord>, StoreError>;
}
