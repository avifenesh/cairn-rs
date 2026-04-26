use async_trait::async_trait;
use cairn_domain::{RunId, ToolInvocationId};

use crate::error::StoreError;

pub use cairn_domain::tool_invocation::{ToolInvocationRecord, ToolInvocationState};

use cairn_domain::{ProjectKey, TaskId};

/// F52: projected row of `ToolInvocationCacheHit`. One per cache-hit
/// event. Operators can count/list via this read model without scanning
/// the event log; backends must write one row per event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolInvocationCacheHitRecord {
    pub invocation_id: ToolInvocationId,
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub tool_name: String,
    pub tool_call_id: String,
    pub original_completed_at_ms: u64,
    pub served_at_ms: u64,
}

/// F52: read-model for cache hits. Optional by design — infra that only
/// cares about execution state can keep ignoring the table.
#[async_trait]
pub trait ToolInvocationCacheHitReadModel: Send + Sync {
    /// List cache hits for a run in served-at order (newest first by
    /// default at the backend level).
    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<ToolInvocationCacheHitRecord>, StoreError>;

    /// Count cache hits for a run.
    async fn count_by_run(&self, run_id: &RunId) -> Result<usize, StoreError>;
}

/// Read-model for tool invocation current state.
#[async_trait]
pub trait ToolInvocationReadModel: Send + Sync {
    async fn get(
        &self,
        invocation_id: &ToolInvocationId,
    ) -> Result<Option<ToolInvocationRecord>, StoreError>;

    /// List tool invocations for a run (timeline view).
    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ToolInvocationRecord>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::ToolInvocationState;

    #[test]
    fn terminal_states_are_correct() {
        assert!(ToolInvocationState::Completed.is_terminal());
        assert!(ToolInvocationState::Failed.is_terminal());
        assert!(ToolInvocationState::Canceled.is_terminal());
        assert!(!ToolInvocationState::Requested.is_terminal());
        assert!(!ToolInvocationState::Started.is_terminal());
    }
}
