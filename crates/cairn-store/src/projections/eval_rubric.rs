use async_trait::async_trait;
use cairn_domain::{EvalRubric, TenantId};

use crate::error::StoreError;

#[async_trait]
pub trait EvalRubricReadModel: Send + Sync {
    async fn get_rubric(&self, rubric_id: &str) -> Result<Option<EvalRubric>, StoreError>;

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<EvalRubric>, StoreError>;
}
