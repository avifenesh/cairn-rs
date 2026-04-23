use async_trait::async_trait;
use cairn_domain::{AuditLogEntry, TenantId};

use crate::error::StoreError;

#[async_trait]
pub trait AuditLogReadModel: Send + Sync {
    /// List audit-log entries for a tenant, newest-first.
    ///
    /// `since_ms` / `before_ms` bound the `occurred_at_ms` window
    /// (inclusive lower / exclusive upper — `[since, before)`). Either may be
    /// `None` to leave that side unbounded. `limit` caps the returned count.
    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        since_ms: Option<u64>,
        before_ms: Option<u64>,
        limit: usize,
    ) -> Result<Vec<AuditLogEntry>, StoreError>;

    async fn list_by_resource(
        &self,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<Vec<AuditLogEntry>, StoreError>;
}
