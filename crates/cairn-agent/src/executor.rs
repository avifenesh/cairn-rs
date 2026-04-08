//! Agent executor — drives the ReAct loop on top of the runtime spine.
//!
//! The executor coordinates:
//! 1. Session + run creation via runtime hooks
//! 2. Prompt resolution via the eval selector resolver
//! 3. Iterative ReAct steps (observe → think → act)
//! 4. Subagent spawning when needed
//! 5. Completion/failure reporting back to the runtime

use async_trait::async_trait;
use cairn_domain::{RunId, SessionId};

use crate::hooks::{HookError, PromptResolver, RuntimeHookHandler};
use crate::orchestrator::{AgentConfig, StepContext, StepOutcome};
use crate::reflection::ReflectionAdvisory;

/// Drives a single agent step. Concrete implementations provide the
/// model call, tool dispatch, and observation logic.
#[async_trait]
pub trait AgentDriver: Send + Sync {
    /// Execute one step of the agent loop given the current context.
    async fn step(&self, ctx: &StepContext) -> Result<StepOutcome, AgentError>;

    /// Reflect on progress after a step.
    async fn reflect(&self, ctx: &StepContext, outcome: &StepOutcome) -> ReflectionAdvisory;
}

/// Agent execution error.
#[derive(Debug)]
pub enum AgentError {
    Hook(HookError),
    Driver(String),
    MaxIterations { limit: u32 },
    Timeout,
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentError::Hook(e) => write!(f, "hook error: {e}"),
            AgentError::Driver(msg) => write!(f, "driver error: {msg}"),
            AgentError::MaxIterations { limit } => {
                write!(f, "max iterations reached: {limit}")
            }
            AgentError::Timeout => write!(f, "agent execution timed out"),
        }
    }
}

impl std::error::Error for AgentError {}

impl From<HookError> for AgentError {
    fn from(e: HookError) -> Self {
        AgentError::Hook(e)
    }
}

/// Result of a complete agent execution.
#[derive(Clone, Debug)]
pub struct ExecutionResult {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub iterations: u32,
    pub final_outcome: StepOutcome,
}

/// The main agent executor. Drives the ReAct loop by coordinating
/// the driver, runtime hooks, and prompt resolver.
pub struct AgentExecutor<H, P, D> {
    hooks: H,
    prompt_resolver: P,
    driver: D,
}

impl<H, P, D> AgentExecutor<H, P, D>
where
    H: RuntimeHookHandler,
    P: PromptResolver,
    D: AgentDriver,
{
    pub fn new(hooks: H, prompt_resolver: P, driver: D) -> Self {
        Self {
            hooks,
            prompt_resolver,
            driver,
        }
    }

    /// Execute an agent with the given configuration.
    pub async fn execute(
        &self,
        config: AgentConfig,
        session_id: SessionId,
        run_id: RunId,
    ) -> Result<ExecutionResult, AgentError> {
        let max_iterations = config.max_iterations.unwrap_or(100);

        // Notify runtime of execution start
        self.hooks
            .on_execution_starting(&config, &session_id, &run_id)
            .await?;

        // Resolve prompt for this agent context
        let resolved_prompt = self
            .prompt_resolver
            .resolve(
                &config.project,
                &cairn_evals::ResolutionContext {
                    agent_type: Some(config.agent_type.as_str().to_owned()),
                    task_type: None,
                    routing_slot: None,
                },
            )
            .await?;

        let mut ctx = StepContext {
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            current_task_id: None,
            iteration: 0,
            resolved_prompt,
        };

        // ReAct loop
        loop {
            ctx.iteration += 1;

            if ctx.iteration > max_iterations {
                self.hooks
                    .on_execution_failed(&run_id, "max iterations reached")
                    .await?;
                return Err(AgentError::MaxIterations {
                    limit: max_iterations,
                });
            }

            let outcome = self
                .driver
                .step(&ctx)
                .await
                .map_err(|e| AgentError::Driver(e.to_string()))?;

            // Notify runtime of step completion
            self.hooks
                .on_step_completed(&run_id, ctx.iteration, &outcome)
                .await?;

            // Reflect
            let advisory = self.driver.reflect(&ctx, &outcome).await;
            if let ReflectionAdvisory::Escalate { reason } = advisory {
                self.hooks.on_execution_failed(&run_id, &reason).await?;
                return Ok(ExecutionResult {
                    session_id,
                    run_id,
                    iterations: ctx.iteration,
                    final_outcome: StepOutcome::Failed { reason },
                });
            }

            // Check outcome
            match &outcome {
                StepOutcome::Done => {
                    self.hooks.on_execution_completed(&run_id).await?;
                    return Ok(ExecutionResult {
                        session_id,
                        run_id,
                        iterations: ctx.iteration,
                        final_outcome: outcome,
                    });
                }
                StepOutcome::Failed { reason } => {
                    self.hooks.on_execution_failed(&run_id, reason).await?;
                    return Ok(ExecutionResult {
                        session_id,
                        run_id,
                        iterations: ctx.iteration,
                        final_outcome: outcome,
                    });
                }
                StepOutcome::SpawnSubagent { agent_type } => {
                    let spawn_request = crate::subagents::SpawnRequest {
                        parent_run_id: run_id.clone(),
                        parent_task_id: None,
                        agent_type: agent_type.clone(),
                        project: config.project.clone(),
                        block_parent: true,
                    };
                    self.hooks.on_subagent_requested(&spawn_request).await?;
                    // Continue loop — runtime handles dependency waiting
                }
                StepOutcome::Continue
                | StepOutcome::ToolCall { .. }
                | StepOutcome::WaitApproval => {
                    // Continue the loop
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_result_carries_metadata() {
        let result = ExecutionResult {
            session_id: SessionId::new("s1"),
            run_id: RunId::new("r1"),
            iterations: 5,
            final_outcome: StepOutcome::Done,
        };
        assert_eq!(result.iterations, 5);
        assert!(matches!(result.final_outcome, StepOutcome::Done));
    }

    #[test]
    fn agent_error_from_hook() {
        let hook_err = HookError::Runtime("test".into());
        let err: AgentError = hook_err.into();
        assert!(matches!(err, AgentError::Hook(_)));
    }
}
