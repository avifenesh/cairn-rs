//! Provider health service boundary for RFC 009 failover handling.

use async_trait::async_trait;
use cairn_domain::providers::{ProviderHealthRecord, ProviderHealthSchedule};
use cairn_domain::{ProviderConnectionId, TenantId};

use crate::error::RuntimeError;

#[async_trait]
pub trait ProviderHealthService: Send + Sync {
    async fn record_check(
        &self,
        connection_id: &ProviderConnectionId,
        latency_ms: u64,
        success: bool,
    ) -> Result<ProviderHealthRecord, RuntimeError>;

    async fn mark_recovered(
        &self,
        connection_id: &ProviderConnectionId,
    ) -> Result<ProviderHealthRecord, RuntimeError>;

    async fn get(
        &self,
        connection_id: &ProviderConnectionId,
    ) -> Result<Option<ProviderHealthRecord>, RuntimeError>;

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProviderHealthRecord>, RuntimeError>;

    /// Schedule periodic health checks for a provider connection.
    async fn schedule_health_check(
        &self,
        connection_id: &ProviderConnectionId,
        interval_ms: u64,
    ) -> Result<ProviderHealthSchedule, RuntimeError>;

    /// Run all due health checks and return the resulting health records.
    async fn run_due_health_checks(&self) -> Result<Vec<ProviderHealthRecord>, RuntimeError>;
}
