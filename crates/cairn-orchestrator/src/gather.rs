//! GatherPhase trait — collects context for a single decision step.

use async_trait::async_trait;

use crate::context::{GatherOutput, OrchestrationContext};
use crate::error::OrchestratorError;

/// Collects all context needed for a single LLM decision step.
///
/// Implementations pull from:
/// - `cairn_memory::RetrievalService`    — semantic + lexical memory search
/// - `cairn_store::EventLog`             — recent events for the current run
/// - `cairn_graph::GraphQueryService`    — execution/provenance neighbourhood
/// - `DefaultsReadModel::list_by_scope`  — operator settings
/// - `CheckpointService::latest_for_run` — most recent checkpoint
#[async_trait]
pub trait GatherPhase: Send + Sync {
    /// Collect context for one orchestration step.
    async fn gather(
        &self,
        ctx: &OrchestrationContext,
    ) -> Result<GatherOutput, OrchestratorError>;
}
