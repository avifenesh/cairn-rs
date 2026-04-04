use async_trait::async_trait;
use cairn_domain::{RetentionPolicy, RetentionResult, TenantId};

use crate::error::RuntimeError;

#[async_trait]
pub trait RetentionService: Send + Sync {
    async fn set_policy(
        &self,
        tenant_id: TenantId,
        full_history_days: u32,
        current_state_days: u32,
        max_events_per_entity: u32,
    ) -> Result<RetentionPolicy, RuntimeError>;

    async fn get_policy(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Option<RetentionPolicy>, RuntimeError>;

    async fn apply_retention(&self, tenant_id: &TenantId) -> Result<RetentionResult, RuntimeError>;
}
