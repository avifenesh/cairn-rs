//! Credential service boundary for tenant-scoped encrypted credential storage.

use async_trait::async_trait;
use cairn_domain::credentials::{CredentialRecord, CredentialRotationRecord};
use cairn_domain::{CredentialId, TenantId};

use crate::error::RuntimeError;

#[async_trait]
pub trait CredentialService: Send + Sync {
    async fn store(
        &self,
        tenant_id: TenantId,
        provider_id: String,
        plaintext_value: String,
        key_id: Option<String>,
    ) -> Result<CredentialRecord, RuntimeError>;

    async fn get(&self, id: &CredentialId) -> Result<Option<CredentialRecord>, RuntimeError>;

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<CredentialRecord>, RuntimeError>;

    async fn revoke(&self, id: &CredentialId) -> Result<CredentialRecord, RuntimeError>;

    async fn rotate_key(
        &self,
        tenant_id: TenantId,
        old_key_id: String,
        new_key_id: String,
    ) -> Result<CredentialRotationRecord, RuntimeError>;
}
