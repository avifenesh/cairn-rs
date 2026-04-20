use crate::{ProjectKey, TaskId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDependency {
    pub dependent_task_id: TaskId,
    pub depends_on_task_id: TaskId,
    pub project: ProjectKey,
    pub created_at_ms: u64,
}

/// Wire shape for HTTP responses on the dependency routes.
///
/// Under the FF-authoritative model (see `docs/design/CAIRN-FABRIC-FINALIZED.md`)
/// cairn does not persist dependency records — FF owns edge state via
/// `ff_stage_dependency_edge` / `ff_apply_dependency_to_child`. This
/// struct is synthesized by the service layer when responding to
/// `declare_dependency` / `check_dependencies` calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDependencyRecord {
    pub dependency: TaskDependency,
    pub resolved_at_ms: Option<u64>,
}
