//! Tenant service boundary for org hierarchy management.

use async_trait::async_trait;
use cairn_domain::{TenantId, TenantRecord};

use crate::error::RuntimeError;

/// Tenant service boundary.
///
/// Manages tenant lifecycle within the organization hierarchy.
#[async_trait]
pub trait TenantService: Send + Sync {
    /// Create a new tenant.
    async fn create(&self, tenant_id: TenantId, name: String)
        -> Result<TenantRecord, RuntimeError>;

    /// Get a tenant by ID.
    async fn get(&self, tenant_id: &TenantId) -> Result<Option<TenantRecord>, RuntimeError>;

    /// List tenants with pagination.
    async fn list(&self, limit: usize, offset: usize) -> Result<Vec<TenantRecord>, RuntimeError>;
}
