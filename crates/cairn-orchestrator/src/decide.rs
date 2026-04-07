//! DecidePhase trait — calls the LLM with gathered context and returns proposals.

use async_trait::async_trait;

use crate::context::{DecideOutput, GatherOutput, OrchestrationContext};
use crate::error::OrchestratorError;

/// Calls the brain LLM with gathered context and parses the response into
/// a set of `ActionProposal` values.
///
/// Implementations should:
/// 1. Resolve the system prompt via `PromptResolver` for `ctx.agent_type`.
/// 2. Build the conversation: system prompt + memory chunks + step history + goal.
/// 3. Call `GenerationProvider::generate` on the brain tier.
/// 4. Parse the response into `Vec<ActionProposal>` (structured output / JSON).
/// 5. Apply `ConfidenceCalibrator` to adjust `predicted_confidence`.
/// 6. Set `requires_approval` when any proposal has `ExecutionClass::Sensitive`.
#[async_trait]
pub trait DecidePhase: Send + Sync {
    /// Produce a set of proposed actions from the gathered context.
    async fn decide(
        &self,
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
    ) -> Result<DecideOutput, OrchestratorError>;
}
