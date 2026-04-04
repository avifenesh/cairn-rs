use async_trait::async_trait;
use cairn_domain::{EvalBaseline, TenantId};

use crate::error::StoreError;

#[async_trait]
pub trait EvalBaselineReadModel: Send + Sync {
    async fn get_baseline(&self, baseline_id: &str) -> Result<Option<EvalBaseline>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<EvalBaseline>, StoreError>;
}
