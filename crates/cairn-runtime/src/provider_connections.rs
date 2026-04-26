//! Provider connection service boundary for tenant-scoped provider setup.

use async_trait::async_trait;
use cairn_domain::providers::ProviderConnectionRecord;
use cairn_domain::{ProviderConnectionId, TenantId};

use crate::error::RuntimeError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderConnectionConfig {
    pub provider_family: String,
    pub adapter_type: String,
    /// Model identifiers served through this connection (e.g. ["gemma4", "qwen3.5"]).
    pub supported_models: Vec<String>,
}

#[async_trait]
pub trait ProviderConnectionService: Send + Sync {
    async fn create(
        &self,
        tenant_id: TenantId,
        provider_connection_id: ProviderConnectionId,
        config: ProviderConnectionConfig,
    ) -> Result<ProviderConnectionRecord, RuntimeError>;

    async fn get(
        &self,
        id: &ProviderConnectionId,
    ) -> Result<Option<ProviderConnectionRecord>, RuntimeError>;

    async fn update(
        &self,
        id: &ProviderConnectionId,
        config: ProviderConnectionConfig,
    ) -> Result<ProviderConnectionRecord, RuntimeError>;

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProviderConnectionRecord>, RuntimeError>;

    /// Hard-delete a provider connection. The projection row is removed
    /// so the `provider_connection_id` is available for re-creation;
    /// the historical `ProviderConnectionRegistered` event stays in the
    /// log for audit. Returns `NotFound` if the connection does not
    /// exist. F40.
    async fn delete(&self, id: &ProviderConnectionId) -> Result<(), RuntimeError>;
}
