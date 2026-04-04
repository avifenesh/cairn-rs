use async_trait::async_trait;
use cairn_domain::{AuditLogEntry, TenantId};

use crate::error::StoreError;

#[async_trait]
pub trait AuditLogReadModel: Send + Sync {
    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        since_ms: Option<u64>,
        limit: usize,
    ) -> Result<Vec<AuditLogEntry>, StoreError>;

    async fn list_by_resource(
        &self,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<Vec<AuditLogEntry>, StoreError>;
}
