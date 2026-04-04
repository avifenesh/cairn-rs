use async_trait::async_trait;
use cairn_domain::RunId;
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperatorInterventionRecord {
    pub run_id: RunId,
    pub tenant_id: cairn_domain::TenantId,
    pub action: String,
    pub reason: String,
    pub intervened_at_ms: u64,
}

#[async_trait]
pub trait OperatorInterventionReadModel: Send + Sync {
    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<OperatorInterventionRecord>, StoreError>;
}
