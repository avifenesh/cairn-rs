use async_trait::async_trait;
use cairn_domain::{IngestJobId, IngestJobRecord, ProjectKey};

use crate::error::StoreError;

/// Read-model for ingest job current state.
#[async_trait]
pub trait IngestJobReadModel: Send + Sync {
    async fn get(&self, job_id: &IngestJobId) -> Result<Option<IngestJobRecord>, StoreError>;

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<IngestJobRecord>, StoreError>;
}
