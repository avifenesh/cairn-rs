use async_trait::async_trait;
use cairn_domain::{RunId, ToolInvocationId};

use crate::error::StoreError;

pub use cairn_domain::tool_invocation::{ToolInvocationRecord, ToolInvocationState};

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
