use async_trait::async_trait;
use cairn_domain::{compaction::Snapshot, TenantId};

use crate::error::StoreError;

/// Read model for tenant snapshots.
#[async_trait]
pub trait SnapshotReadModel: Send + Sync {
    async fn get_latest(&self, tenant_id: &TenantId) -> Result<Option<Snapshot>, StoreError>;
    async fn list_by_tenant(&self, tenant_id: &TenantId) -> Result<Vec<Snapshot>, StoreError>;
}
