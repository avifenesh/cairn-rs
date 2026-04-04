use async_trait::async_trait;
use cairn_domain::providers::{
    OperationKind, ProviderBindingCostStats, ProviderBindingRecord, ProviderBudget,
    ProviderBudgetPeriod, ProviderConnectionPool, ProviderConnectionRecord, ProviderHealthRecord,
    ProviderHealthSchedule, ProviderModelCapability, RoutePolicy, RunCostAlert, RunCostRecord,
    SessionCostRecord,
};
use cairn_domain::{
    ProjectKey, ProviderBindingId, ProviderConnectionId, RunId, SessionId, TenantId,
};

use crate::error::StoreError;

#[async_trait]
pub trait ProviderConnectionReadModel: Send + Sync {
    async fn get(
        &self,
        id: &ProviderConnectionId,
    ) -> Result<Option<ProviderConnectionRecord>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProviderConnectionRecord>, StoreError>;
}

#[async_trait]
pub trait ProviderBindingReadModel: Send + Sync {
    async fn get(
        &self,
        id: &ProviderBindingId,
    ) -> Result<Option<ProviderBindingRecord>, StoreError>;

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProviderBindingRecord>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProviderBindingRecord>, StoreError>;

    async fn list_active(
        &self,
        project: &ProjectKey,
        operation: OperationKind,
    ) -> Result<Vec<ProviderBindingRecord>, StoreError>;
}

#[async_trait]
pub trait ProviderHealthReadModel: Send + Sync {
    async fn get(
        &self,
        connection_id: &ProviderConnectionId,
    ) -> Result<Option<ProviderHealthRecord>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProviderHealthRecord>, StoreError>;
}

#[async_trait]
pub trait RoutePolicyReadModel: Send + Sync {
    async fn get(&self, policy_id: &str) -> Result<Option<RoutePolicy>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RoutePolicy>, StoreError>;
}

#[async_trait]
pub trait SessionCostReadModel: Send + Sync {
    async fn get_session_cost(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionCostRecord>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        since_ms: u64,
    ) -> Result<Vec<SessionCostRecord>, StoreError>;
}

#[async_trait]
pub trait RunCostReadModel: Send + Sync {
    async fn get_run_cost(&self, run_id: &RunId) -> Result<Option<RunCostRecord>, StoreError>;

    async fn list_by_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<RunCostRecord>, StoreError>;
}

/// RFC 010: read-model for run cost alert thresholds and triggered alerts.
#[async_trait]
pub trait RunCostAlertReadModel: Send + Sync {
    async fn get_alert(&self, run_id: &RunId) -> Result<Option<RunCostAlert>, StoreError>;
    async fn list_triggered_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<RunCostAlert>, StoreError>;
}

#[async_trait]
pub trait ProviderHealthScheduleReadModel: Send + Sync {
    async fn get_schedule(
        &self,
        schedule_id: &str,
    ) -> Result<Option<ProviderHealthSchedule>, StoreError>;

    async fn list_schedules_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ProviderHealthSchedule>, StoreError>;

    async fn list_enabled_schedules(&self) -> Result<Vec<ProviderHealthSchedule>, StoreError>;
}

/// Read-model for the provider model capability registry (RFC 009).
#[async_trait]
pub trait ProviderModelReadModel: Send + Sync {
    /// Get capabilities for a specific model_id.
    async fn get_model(&self, model_id: &str) -> Result<Option<ProviderModelCapability>, StoreError>;

    /// List all registered models for a connection.
    async fn list_by_connection(
        &self,
        connection_id: &ProviderConnectionId,
    ) -> Result<Vec<ProviderModelCapability>, StoreError>;
}

/// RFC 009: read-model for per-binding actual cost statistics.
#[async_trait]
pub trait ProviderBindingCostStatsReadModel: Send + Sync {
    async fn get(
        &self,
        binding_id: &ProviderBindingId,
    ) -> Result<Option<ProviderBindingCostStats>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ProviderBindingCostStats>, StoreError>;
}

#[async_trait]
pub trait ProviderBudgetReadModel: Send + Sync {
    async fn get_by_tenant_period(
        &self,
        tenant_id: &TenantId,
        period: ProviderBudgetPeriod,
    ) -> Result<Option<ProviderBudget>, StoreError>;

    async fn list_by_tenant(&self, tenant_id: &TenantId)
        -> Result<Vec<ProviderBudget>, StoreError>;
}

/// RFC 009: read-model for provider connection pools.
#[async_trait]
pub trait ProviderPoolReadModel: Send + Sync {
    async fn get_pool(&self, pool_id: &str) -> Result<Option<ProviderConnectionPool>, StoreError>;
    async fn list_pools_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ProviderConnectionPool>, StoreError>;
}
