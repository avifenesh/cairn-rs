//! Subagent spawn and linkage boundaries per RFC 005.
//!
//! Subagent execution is represented explicitly:
//! - spawning creates a child task + child session
//! - child run is created when task transitions to running
//! - parent run enters waiting_dependency if blocked

use cairn_domain::{ProjectKey, RunId, SessionId, TaskId};
use serde::{Deserialize, Serialize};

use crate::orchestrator::AgentType;

/// Request to spawn a subagent from a parent run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpawnRequest {
    pub parent_run_id: RunId,
    pub parent_task_id: Option<TaskId>,
    pub agent_type: AgentType,
    pub project: ProjectKey,
    pub block_parent: bool,
}

/// Linkage record created after subagent spawn.
///
/// Per RFC 005, required linkage fields:
/// - parent_run_id
/// - parent_task_id (where applicable)
/// - child_task_id
/// - child_session_id
/// - child_run_id (if created immediately)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubagentLink {
    pub parent_run_id: RunId,
    pub parent_task_id: Option<TaskId>,
    pub child_task_id: TaskId,
    pub child_session_id: SessionId,
    pub child_run_id: Option<RunId>,
}

/// Outcome reported by a completed subagent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum SubagentOutcome {
    Completed,
    Failed { reason: String },
    Canceled,
}

#[cfg(test)]
mod tests {
    use super::SubagentOutcome;

    #[test]
    fn subagent_outcomes_are_distinct() {
        assert_ne!(SubagentOutcome::Completed, SubagentOutcome::Canceled);
    }
}
