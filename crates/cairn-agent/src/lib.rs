//! Agent runtime, orchestration, and subagent execution boundaries.
//!
//! `cairn-agent` owns the agent execution model:
//!
//! - **Orchestrator**: high-level agent execution coordination
//! - **React**: ReAct (Reason + Act) loop step types and control
//! - **Subagents**: spawn/link types for child task/session creation (RFC 005)
//! - **Reflection**: self-inspection and advisory signals

pub mod executor;
pub mod hooks;
pub mod orchestrator;
pub mod react;
pub mod reflection;
pub mod streaming;
pub mod subagents;

pub use executor::{AgentDriver, AgentError, AgentExecutor, ExecutionResult};
pub use hooks::{HookError, PromptResolver, RuntimeHook, RuntimeHookHandler, SubagentSpawnResult};
pub use orchestrator::{AgentConfig, AgentType, ResolvedPrompt, StepContext, StepOutcome};
pub use react::{LoopSignal, ReactPhase};
pub use reflection::ReflectionAdvisory;
pub use streaming::{
    AssistantDelta, AssistantEnd, AssistantReasoning, StopReason, StreamingOutput,
};
pub use subagents::{SpawnRequest, SubagentLink, SubagentOutcome};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles_with_domain_dependency() {
        let id = cairn_domain::SessionId::new("test");
        assert_eq!(id.as_str(), "test");
    }
}
