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

pub use context::{
    ActionResult, ActionStatus, CompactionConfig, DecideOutput, ExecuteOutcome, GatherOutput,
    LoopConfig, LoopSignal, LoopTermination, OrchestrationContext, StepSummary,
};
pub use decide::DecidePhase;
pub use decide_impl::{estimate_tokens, LlmDecidePhase, TokenBudget};
pub use emitter::{ChannelEmitter, NoOpEmitter, OrchestratorEvent, OrchestratorEventEmitter};
pub use error::OrchestratorError;
pub use execute::ExecutePhase;
pub use gather::GatherPhase;
pub use loop_runner::{
    CheckpointHook, DualCheckpointHook, NoOpCheckpointHook, OrchestratorLoop,
    LEASE_UNHEALTHY_REASON,
};
pub use approval_policy::derive_match_policy;
pub use task_sink::{NoOpTaskSink, TaskFrameSink};

pub mod gather_impl;
pub use gather_impl::StandardGatherPhase;

pub mod execute_impl;
pub use execute_impl::RuntimeExecutePhase;
