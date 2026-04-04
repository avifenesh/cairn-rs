use async_trait::async_trait;
use cairn_domain::{RunId, TenantId, sla::{SlaConfig, SlaBreach}};

use crate::error::StoreError;

/// Read model for run SLA configurations and breach records.
#[async_trait]
pub trait RunSlaReadModel: Send + Sync {
    async fn get_sla(&self, run_id: &RunId) -> Result<Option<SlaConfig>, StoreError>;
    async fn get_breach(&self, run_id: &RunId) -> Result<Option<SlaBreach>, StoreError>;
    async fn list_breached_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<SlaBreach>, StoreError>;
}
