use async_trait::async_trait;
use cairn_domain::{TenantId, TenantQuota};

use crate::error::RuntimeError;

#[async_trait]
pub trait QuotaService: Send + Sync {
    async fn set_quota(
        &self,
        tenant_id: TenantId,
        max_concurrent_runs: u32,
        max_sessions_per_hour: u32,
        max_tasks_per_run: u32,
    ) -> Result<TenantQuota, RuntimeError>;

    async fn get_quota(&self, tenant_id: &TenantId) -> Result<Option<TenantQuota>, RuntimeError>;

    async fn check_run_quota(&self, tenant_id: &TenantId) -> Result<(), RuntimeError>;

    async fn check_session_quota(&self, tenant_id: &TenantId) -> Result<(), RuntimeError>;
}
