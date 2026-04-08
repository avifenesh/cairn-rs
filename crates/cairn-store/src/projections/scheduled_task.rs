use async_trait::async_trait;
use cairn_domain::{ScheduledTaskId, ScheduledTaskRecord, TenantId};

use crate::error::StoreError;

/// Read-model for scheduled task current state.
#[async_trait]
pub trait ScheduledTaskReadModel: Send + Sync {
    async fn get(&self, id: &ScheduledTaskId) -> Result<Option<ScheduledTaskRecord>, StoreError>;

    /// List all scheduled tasks for a tenant, enabled or not.
    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ScheduledTaskRecord>, StoreError>;

    /// List only enabled tasks whose `next_run_at` is at or before `now_ms`.
    /// Used by the recovery sweep to find tasks due for execution.
    async fn list_due(
        &self,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<ScheduledTaskRecord>, StoreError>;
}
