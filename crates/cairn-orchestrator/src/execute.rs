//! ExecutePhase trait — runs approved actions through the runtime pipeline.

use async_trait::async_trait;

use crate::context::{DecideOutput, ExecuteOutcome, OrchestrationContext};
use crate::error::OrchestratorError;

/// Dispatches each `ActionProposal` from `DecideOutput` through the
/// appropriate runtime service.
///
/// Dispatch table (by `ActionType`):
///
/// | `ActionType`        | Service used                                       |
/// |---------------------|----------------------------------------------------|
/// | `InvokeTool`        | `ToolInvocationService` + tool registry dispatch   |
/// | `SpawnSubagent`     | `TaskServiceImpl::spawn_subagent`                  |
/// | `SendNotification`  | `MailboxService::send`                             |
/// | `CompleteRun`       | `RunService::complete`                             |
/// | `EscalateToOperator`| `ApprovalService::request` (sets requires_approval)|
/// | `CreateMemory`      | `IngestService::submit`                            |
///
/// After each successful tool call, `CheckpointService::save` is called
/// per the `LoopConfig::checkpoint_every_n_tool_calls` policy.
///
/// The returned `ExecuteOutcome::loop_signal` tells `OrchestratorLoop`
/// whether to continue, suspend, or terminate.
#[async_trait]
pub trait ExecutePhase: Send + Sync {
    /// Execute all approved proposals and return the combined outcome.
    async fn execute(
        &self,
        ctx: &OrchestrationContext,
        decide: &DecideOutput,
    ) -> Result<ExecuteOutcome, OrchestratorError>;
}
