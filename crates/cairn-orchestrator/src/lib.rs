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

pub mod context;
pub mod error;
pub mod gather;
pub mod decide;
pub mod decide_impl;
pub mod execute;
pub mod loop_runner;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use context::{
    ActionResult, ActionStatus, DecideOutput, ExecuteOutcome, GatherOutput,
    LoopConfig, LoopSignal, LoopTermination, OrchestrationContext, StepSummary,
};
pub use error::OrchestratorError;
pub use gather::GatherPhase;
pub use decide::DecidePhase;
pub use decide_impl::LlmDecidePhase;
pub use execute::ExecutePhase;
pub use loop_runner::OrchestratorLoop;

pub mod gather_impl;
pub use gather_impl::StandardGatherPhase;

pub mod execute_impl;
pub use execute_impl::RuntimeExecutePhase;
