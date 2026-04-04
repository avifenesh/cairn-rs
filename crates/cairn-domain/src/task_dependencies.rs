use crate::{ProjectKey, TaskId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDependency {
    pub dependent_task_id: TaskId,
    pub depends_on_task_id: TaskId,
    pub project: ProjectKey,
    pub created_at_ms: u64,
}
