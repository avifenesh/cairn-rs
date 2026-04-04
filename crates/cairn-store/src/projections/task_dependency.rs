use async_trait::async_trait;
use cairn_domain::{ProjectKey, TaskDependency, TaskId};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskDependencyRecord {
    pub dependency: TaskDependency,
    pub resolved_at_ms: Option<u64>,
}

#[async_trait]
pub trait TaskDependencyReadModel: Send + Sync {
    async fn list_blocking(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<TaskDependencyRecord>, StoreError>;

    async fn list_unresolved(
        &self,
        project: &ProjectKey,
    ) -> Result<Vec<TaskDependencyRecord>, StoreError>;
}
