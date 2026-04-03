//! Agent-runtime hooks connecting the orchestrator to runtime services.
//!
//! These traits define the boundary between the agent execution layer
//! and the runtime spine. The agent orchestrator calls these hooks to
//! manage sessions, runs, and tasks without depending directly on the
//! runtime service implementations.

use async_trait::async_trait;
use cairn_domain::{ProjectKey, RunId, SessionId, TaskId};
use cairn_evals::selectors::ResolutionContext;
use serde::{Deserialize, Serialize};

use crate::orchestrator::{AgentConfig, ResolvedPrompt, StepOutcome};
use crate::subagents::SpawnRequest;

/// Lifecycle events the orchestrator emits for the runtime to handle.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "hook", rename_all = "snake_case")]
pub enum RuntimeHook {
    /// Agent execution is starting. Runtime should create session + run.
    ExecutionStarting {
        config: AgentConfig,
        session_id: SessionId,
        run_id: RunId,
    },
    /// Agent step completed. Runtime records progress.
    StepCompleted {
        run_id: RunId,
        iteration: u32,
        outcome: StepOutcome,
    },
    /// Agent wants to spawn a subagent. Runtime creates child task + session.
    SubagentRequested { request: SpawnRequest },
    /// Agent execution completed successfully. Runtime completes the run.
    ExecutionCompleted { run_id: RunId },
    /// Agent execution failed. Runtime fails the run.
    ExecutionFailed { run_id: RunId, reason: String },
}

/// Runtime hook handler that the orchestrator calls during execution.
///
/// Concrete implementations wire these to the actual runtime services
/// (SessionService, RunService, TaskService).
#[async_trait]
pub trait RuntimeHookHandler: Send + Sync {
    /// Called before agent execution begins.
    async fn on_execution_starting(
        &self,
        config: &AgentConfig,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<(), HookError>;

    /// Called after each agent step.
    async fn on_step_completed(
        &self,
        run_id: &RunId,
        iteration: u32,
        outcome: &StepOutcome,
    ) -> Result<(), HookError>;

    /// Called when the agent wants to spawn a subagent.
    async fn on_subagent_requested(
        &self,
        request: &SpawnRequest,
    ) -> Result<SubagentSpawnResult, HookError>;

    /// Called when agent execution completes.
    async fn on_execution_completed(&self, run_id: &RunId) -> Result<(), HookError>;

    /// Called when agent execution fails.
    async fn on_execution_failed(&self, run_id: &RunId, reason: &str) -> Result<(), HookError>;
}

/// Result from subagent spawn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubagentSpawnResult {
    pub child_task_id: TaskId,
    pub child_session_id: SessionId,
}

/// Prompt resolution hook for the agent orchestrator.
///
/// Resolves which prompt release to use for a given agent execution
/// context, delegating to the eval crate's selector resolver.
#[async_trait]
pub trait PromptResolver: Send + Sync {
    /// Resolve the prompt for an agent execution context.
    async fn resolve(
        &self,
        project: &ProjectKey,
        ctx: &ResolutionContext,
    ) -> Result<Option<ResolvedPrompt>, HookError>;
}

#[derive(Debug)]
pub enum HookError {
    Runtime(String),
    PromptResolution(String),
    Internal(String),
}

impl std::fmt::Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookError::Runtime(msg) => write!(f, "runtime hook error: {msg}"),
            HookError::PromptResolution(msg) => write!(f, "prompt resolution error: {msg}"),
            HookError::Internal(msg) => write!(f, "internal hook error: {msg}"),
        }
    }
}

impl std::error::Error for HookError {}

#[cfg(test)]
mod tests {
    use super::{RuntimeHook, SubagentSpawnResult};
    use cairn_domain::{SessionId, TaskId};

    #[test]
    fn hook_variants_are_constructible() {
        let hook = RuntimeHook::ExecutionCompleted {
            run_id: "run_1".into(),
        };
        assert!(matches!(hook, RuntimeHook::ExecutionCompleted { .. }));
    }

    #[test]
    fn spawn_result_carries_ids() {
        let result = SubagentSpawnResult {
            child_task_id: TaskId::new("t1"),
            child_session_id: SessionId::new("s1"),
        };
        assert_eq!(result.child_task_id.as_str(), "t1");
    }
}
