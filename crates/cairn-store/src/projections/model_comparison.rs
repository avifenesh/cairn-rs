use async_trait::async_trait;
use cairn_domain::evals::ModelComparisonRun;
use cairn_domain::TenantId;

use crate::error::StoreError;

/// Read model for model comparison runs.
#[async_trait]
pub trait ModelComparisonReadModel: Send + Sync {
    async fn get_comparison(
        &self,
        comparison_id: &str,
    ) -> Result<Option<ModelComparisonRun>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ModelComparisonRun>, StoreError>;
}
