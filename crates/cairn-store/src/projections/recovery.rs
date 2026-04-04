use async_trait::async_trait;
use cairn_domain::{recovery::RecoveryEscalation, RunId, TenantId};

use crate::error::StoreError;

/// Read model for recovery escalations.
#[async_trait]
pub trait RecoveryEscalationReadModel: Send + Sync {
    async fn get_by_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RecoveryEscalation>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<RecoveryEscalation>, StoreError>;
}
