//! Run SLA service boundary per RFC 005.

use async_trait::async_trait;
use cairn_domain::{RunId, TenantId, sla::{SlaConfig, SlaStatus, SlaBreach}};

use crate::error::RuntimeError;

#[async_trait]
pub trait RunSlaService: Send + Sync {
    /// Configure an SLA target for a run.
    async fn set_sla(
        &self,
        run_id: RunId,
        tenant_id: TenantId,
        target_ms: u64,
        alert_pct: u8,
    ) -> Result<SlaConfig, RuntimeError>;

    /// Check the current SLA status for a run (non-mutating).
    async fn check_sla(&self, run_id: &RunId) -> Result<SlaStatus, RuntimeError>;

    /// Check and emit RunSlaBreached if the SLA is exceeded.
    async fn check_and_breach(&self, run_id: &RunId) -> Result<bool, RuntimeError>;

    /// Get the raw SLA config for a run.
    async fn get_sla(&self, run_id: &RunId) -> Result<Option<SlaConfig>, RuntimeError>;

    /// List all SLA-breached runs for a tenant.
    async fn list_breached_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<SlaBreach>, RuntimeError>;
}
