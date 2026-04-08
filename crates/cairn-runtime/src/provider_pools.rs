//! Provider connection pool service boundary per RFC 009.

use async_trait::async_trait;
use cairn_domain::providers::ProviderConnectionPool;
use cairn_domain::{ProviderConnectionId, TenantId};

use crate::error::RuntimeError;

#[async_trait]
pub trait ProviderConnectionPoolService: Send + Sync {
    async fn create_pool(
        &self,
        tenant_id: TenantId,
        pool_id: String,
        max_connections: u32,
    ) -> Result<ProviderConnectionPool, RuntimeError>;

    async fn add_connection(
        &self,
        pool_id: &str,
        connection_id: ProviderConnectionId,
    ) -> Result<ProviderConnectionPool, RuntimeError>;

    async fn remove_connection(
        &self,
        pool_id: &str,
        connection_id: &ProviderConnectionId,
    ) -> Result<ProviderConnectionPool, RuntimeError>;

    async fn get_pool(&self, pool_id: &str)
        -> Result<Option<ProviderConnectionPool>, RuntimeError>;

    async fn list_pools(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ProviderConnectionPool>, RuntimeError>;

    /// Returns the first available (non-full) connection ID from the pool.
    async fn get_available(
        &self,
        pool_id: &str,
    ) -> Result<Option<ProviderConnectionId>, RuntimeError>;
}
