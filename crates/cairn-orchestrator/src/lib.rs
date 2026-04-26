//! cairn-orchestrator — the GATHER → DECIDE → EXECUTE loop.
//!
//! This crate drives agent execution over the cairn-rs runtime spine.
//! It consumes existing services (runs, tasks, approvals, checkpoints,
//! tool invocations, memory retrieval, graph) and orchestrates them into
//! a coherent GATHER → DECIDE → EXECUTE loop per RFC 002 + RFC 005.
//!
//! # Architecture
//!
//! ```text
//! OrchestratorLoop
//!   ├── GatherPhase  → GatherOutput   (context snapshot)
//!   ├── DecidePhase  → DecideOutput   (proposed actions)
//!   └── ExecutePhase → ExecuteOutcome (actual results)
//! ```
//!
//! All three phases are traits: implementations are injected at construction
//! time so each phase can be tested and replaced independently.

pub mod approval_policy;
pub mod completion_verification;
pub mod context;
pub mod decide;
pub mod decide_impl;
pub mod emitter;
pub mod error;
pub mod execute;
pub mod gather;
pub mod loop_runner;
pub mod task_sink;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use approval_policy::derive_match_policy;
pub use completion_verification::extract_verification;
pub use context::{
    ActionResult, ActionStatus, CompactionConfig, DecideOutput, ExecuteOutcome, GatherOutput,
    LoopConfig, LoopSignal, LoopTermination, OrchestrationContext, StepSummary,
};
pub use decide::DecidePhase;
pub use decide_impl::{estimate_tokens, LlmDecidePhase, TokenBudget};
pub use emitter::{ChannelEmitter, NoOpEmitter, OrchestratorEvent, OrchestratorEventEmitter};
pub use error::OrchestratorError;
// Re-export the runtime's chain/routing types so existing call sites
// (handlers, tests) keep their import paths stable.
pub use cairn_runtime::{
    format_attempt_summary, single_model_service, CooldownMap, FallbackAttempt, FallbackOutcome,
    ModelChain, RoutedBinding, RoutedGenerationError, RoutedGenerationService,
    RoutedGenerationSuccess, DEFAULT_RATE_LIMIT_COOLDOWN,
};
pub use execute::ExecutePhase;
pub use gather::GatherPhase;
pub use loop_runner::{
    CheckpointHook, DualCheckpointHook, NoOpCheckpointHook, OrchestratorLoop,
    LEASE_UNHEALTHY_REASON,
};
pub use task_sink::{NoOpTaskSink, TaskFrameSink};

pub mod gather_impl;
pub use gather_impl::StandardGatherPhase;

pub mod execute_impl;
pub use execute_impl::RuntimeExecutePhase;

/// Test-only helpers exposed for cross-crate regression tests.
///
/// These forward to private prompt builders in `decide_impl.rs` so
/// integration tests (e.g. `cairn-app` F30 termination test) can
/// snapshot the prompt without us having to export the builder itself.
///
/// Gated behind the `test-hooks` Cargo feature so production builds do
/// not expose internal prompt APIs as a stable surface. Test crates
/// opt in with `cairn-orchestrator = { …, features = ["test-hooks"] }`.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub mod decide_impl_test_hooks {
    use crate::context::{GatherOutput, OrchestrationContext};
    use cairn_tools::builtins::BuiltinToolDescriptor;

    /// Build the DECIDE phase system prompt. Forwards to
    /// `decide_impl::build_system_prompt`.
    pub fn build_system_prompt_for_tests(
        agent_type: &str,
        tools: &[BuiltinToolDescriptor],
        native_tools_enabled: bool,
    ) -> String {
        crate::decide_impl::build_system_prompt_pub(agent_type, tools, native_tools_enabled)
    }

    /// Build the DECIDE phase user message. Forwards to
    /// `decide_impl::build_user_message` (which is crate-private).
    ///
    /// F30 uses this to assert the footer no longer reintroduces the
    /// pre-fix four-phase workflow.
    pub fn build_user_message_for_tests(
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
    ) -> String {
        crate::decide_impl::build_user_message_pub(ctx, gather, None)
    }

    /// Return the synthetic `complete_run` OpenAI-format tool schema
    /// that DECIDE injects into every provider call. F36 regression
    /// tests in `cairn-app` assert the shape of this descriptor so a
    /// silent edit can't regress the "model has a first-class
    /// terminal tool" invariant.
    pub fn complete_run_tool_def_for_tests() -> serde_json::Value {
        crate::decide_impl::complete_run_tool_def_pub()
    }

    /// F38 regression hook: decide whether the stuck-loop directive
    /// would be appended to the user message for the given iteration
    /// count and step history.
    pub fn should_inject_stuck_nudge_for_tests(
        iteration: u32,
        step_history: &[crate::context::StepSummary],
    ) -> bool {
        crate::decide_impl::should_inject_stuck_nudge(iteration, step_history)
    }

    /// F38 regression hook: expose the nudge suffix string so tests can
    /// assert (a) that it carries the load-bearing directive substring
    /// and (b) that a future refactor can't silently rewrite it.
    pub fn stuck_nudge_suffix_for_tests() -> &'static str {
        crate::decide_impl::stuck_nudge_suffix()
    }

    /// F38 regression hook: iteration threshold at which the stuck-loop
    /// nudge starts firing.
    pub fn stuck_iteration_threshold_for_tests() -> u32 {
        crate::decide_impl::STUCK_ITERATION_THRESHOLD
    }
}
