use async_trait::async_trait;
use cairn_domain::providers::{ProviderBudget, ProviderBudgetPeriod};
use cairn_domain::TenantId;

use crate::error::RuntimeError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetStatus {
    pub remaining_micros: u64,
    pub percent_used: u32,
    pub alert_triggered: bool,
    pub exceeded: bool,
}

#[async_trait]
pub trait BudgetService: Send + Sync {
    async fn set_budget(
        &self,
        tenant_id: TenantId,
        period: ProviderBudgetPeriod,
        limit_micros: u64,
        alert_threshold_percent: u32,
    ) -> Result<ProviderBudget, RuntimeError>;

    async fn get_budget(
        &self,
        tenant_id: &TenantId,
        period: ProviderBudgetPeriod,
    ) -> Result<Option<ProviderBudget>, RuntimeError>;

    async fn list_budgets(&self, tenant_id: &TenantId)
        -> Result<Vec<ProviderBudget>, RuntimeError>;

    async fn check_budget(&self, tenant_id: &TenantId) -> Result<BudgetStatus, RuntimeError>;
}
