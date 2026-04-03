//! Orchestrator boundaries for agent execution.
//!
//! The orchestrator coordinates the high-level agent execution loop:
//! creating sessions, starting runs, managing tool invocations, and
//! deciding when to spawn subagents or pause for approval.

use cairn_domain::{ProjectKey, PromptAssetId, RunId, SessionId, TaskId};
use serde::{Deserialize, Serialize};

/// Agent type identifier used for prompt selector matching (RFC 006).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentType(pub String);

/// Configuration for an orchestrated agent execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentConfig {
    pub agent_type: AgentType,
    pub project: ProjectKey,
    pub prompt_asset_id: Option<PromptAssetId>,
    pub max_iterations: Option<u32>,
    pub timeout_ms: Option<u64>,
}

/// Outcome of a single agent execution step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum StepOutcome {
    /// Agent produced a response and wants to continue.
    Continue,
    /// Agent needs to invoke a tool before continuing.
    ToolCall { tool_name: String },
    /// Agent wants to spawn a subagent.
    SpawnSubagent { agent_type: AgentType },
    /// Agent is waiting for approval before proceeding.
    WaitApproval,
    /// Agent has completed its work.
    Done,
    /// Agent encountered an error.
    Failed { reason: String },
}

/// Prompt binding resolved at runtime for an agent execution.
///
/// Per RFC 006: runtime usage must record prompt release linkage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedPrompt {
    pub prompt_asset_id: PromptAssetId,
    pub prompt_version_id: cairn_domain::PromptVersionId,
    pub prompt_release_id: Option<cairn_domain::PromptReleaseId>,
}

/// Runtime context snapshot for a single orchestrator step.
#[derive(Clone, Debug)]
pub struct StepContext {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub current_task_id: Option<TaskId>,
    pub iteration: u32,
    pub resolved_prompt: Option<ResolvedPrompt>,
}

impl AgentType {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentType, StepOutcome};

    #[test]
    fn agent_type_preserves_name() {
        let at = AgentType::new("planner");
        assert_eq!(at.as_str(), "planner");
    }

    #[test]
    fn step_outcomes_are_distinct() {
        let done = StepOutcome::Done;
        let cont = StepOutcome::Continue;
        assert_ne!(done, cont);
    }
}
