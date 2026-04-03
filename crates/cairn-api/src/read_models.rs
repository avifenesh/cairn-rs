use cairn_domain::ids::{ApprovalId, RunId, SessionId, TaskId};
use cairn_domain::lifecycle::{RunState, TaskState};
use cairn_domain::policy::ApprovalRequirement;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Operator-facing run summary used by list and detail views.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub project: ProjectKey,
    pub state: RunState,
}

/// Operator-facing task summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSummary {
    pub task_id: TaskId,
    pub project: ProjectKey,
    pub state: TaskState,
    pub parent_run_id: Option<RunId>,
}

/// Operator-facing approval summary for the approval inbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalSummary {
    pub approval_id: ApprovalId,
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub requirement: ApprovalRequirement,
}

/// Seam for operator-facing read models. Implementors query projected state.
pub trait ReadModelQuery {
    type Error;

    fn list_runs(&self, project: &ProjectKey) -> Result<Vec<RunSummary>, Self::Error>;
    fn list_tasks(&self, project: &ProjectKey) -> Result<Vec<TaskSummary>, Self::Error>;
    fn list_approvals(&self, project: &ProjectKey) -> Result<Vec<ApprovalSummary>, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::{ApprovalId, RunId, SessionId};
    use cairn_domain::lifecycle::RunState;
    use cairn_domain::policy::ApprovalRequirement;
    use cairn_domain::tenancy::ProjectKey;

    #[test]
    fn run_summary_construction() {
        let summary = RunSummary {
            run_id: RunId::new("run_1"),
            session_id: SessionId::new("sess_1"),
            project: ProjectKey::new("t", "w", "p"),
            state: RunState::Running,
        };
        assert_eq!(summary.state, RunState::Running);
    }

    #[test]
    fn approval_summary_construction() {
        let summary = ApprovalSummary {
            approval_id: ApprovalId::new("appr_1"),
            project: ProjectKey::new("t", "w", "p"),
            run_id: Some(RunId::new("run_1")),
            task_id: None,
            requirement: ApprovalRequirement::Required,
        };
        assert_eq!(summary.requirement, ApprovalRequirement::Required);
    }
}
