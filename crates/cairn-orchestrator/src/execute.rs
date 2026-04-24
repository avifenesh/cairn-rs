//! ExecutePhase trait — runs approved actions through the runtime pipeline.

use async_trait::async_trait;

use crate::context::{ActionResult, DecideOutput, ExecuteOutcome, OrchestrationContext};
use crate::error::OrchestratorError;

/// F25 drain input: an operator-approved proposal re-hydrated from the
/// `ToolCallApprovalReadModel` projection. Unlike a fresh `ActionProposal`
/// from DECIDE, this carries a pre-minted `ToolCallId` (derived by the
/// execute phase at submission time) so the dispatch must NOT re-derive
/// one — doing so would break the `ToolCallResultCache` keying invariant
/// (RFC 020 Track 3) and double-invoke the tool on replay.
#[derive(Clone, Debug)]
pub struct ApprovedDispatch {
    pub call_id: cairn_domain::ToolCallId,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
}

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

    /// F25 drain entry point: dispatch a tool call whose approval has
    /// already been resolved by the operator. The caller
    /// (`OrchestratorLoop::drain_approved_pending`) supplies the pre-minted
    /// `ToolCallId` + effective args loaded from the projection so the
    /// dispatch bypasses call_id derivation, approval-gate submission,
    /// and `requires_approval` handling — all of which were the original
    /// F25 shadowing bug's escape routes.
    ///
    /// Implementations MUST:
    ///
    /// 1. Skip if `ToolCallResultCache::get(call_id)` hits (already executed).
    /// 2. Record a `ToolInvocationStarted`, dispatch via the registry,
    ///    and record `ToolInvocationCompleted { tool_call_id: Some(...),
    ///    result_json: Some(...) }` on success so the cache replay path
    ///    (startup + future drains) can rebuild this entry.
    /// 3. Populate the shared `ToolCallResultCache` with the result so
    ///    the same-process next-iteration drain observes the hit.
    ///
    /// Default impl returns an `Execute` error so existing test-phase
    /// stubs that don't participate in the drain still compile.
    async fn dispatch_approved(
        &self,
        _ctx: &OrchestrationContext,
        _approved: &ApprovedDispatch,
    ) -> Result<ActionResult, OrchestratorError> {
        Err(OrchestratorError::Execute(
            "dispatch_approved not implemented on this ExecutePhase".to_owned(),
        ))
    }
}
