//! OrchestratorLoop — ties GatherPhase → DecidePhase → ExecutePhase together.
//!
//! The loop drives one run from start to a terminal state (or a suspension
//! point like `waiting_approval` or `waiting_dependency`).  It is the only
//! component that advances the run through the GATHER → DECIDE → EXECUTE
//! cycle, enforces the iteration cap and wall-clock timeout, and records a
//! `StepSummary` checkpoint after each iteration.
//!
//! # Pseudocode (per design doc)
//!
//! ```text
//! loop:
//!   1. check_timeout()
//!   2. gather_output  = gather(ctx)
//!   3. decide_output  = decide(ctx, gather_output)
//!   4. if decide_output.requires_approval → execute(ctx, decide_output) → WaitApproval
//!   5. execute_outcome = execute(ctx, decide_output)
//!   6. checkpoint(ctx, decide_output, execute_outcome)   ← per checkpoint policy
//!   7. match execute_outcome.loop_signal:
//!        Done          → Completed
//!        Failed        → Failed
//!        WaitApproval  → WaitingApproval
//!        WaitSubagent  → WaitingSubagent
//!        Continue      → ctx.iteration += 1, loop
//! max_iterations exceeded → MaxIterationsReached
//! ```

use std::sync::Arc;

use crate::context::{
    ActionStatus, DecideOutput, ExecuteOutcome, GatherOutput, LoopConfig, LoopSignal,
    LoopTermination, OrchestrationContext, StepSummary,
};
use crate::decide::DecidePhase;
use crate::error::OrchestratorError;
use crate::execute::ExecutePhase;
use crate::gather::GatherPhase;

// ── CheckpointHook ────────────────────────────────────────────────────────────

/// Optional hook called after step 5 (execute) to persist iteration state.
///
/// The concrete implementation is injected by the HTTP entry point or test
/// harness. `NoOpCheckpointHook` is used when no durable checkpoint is needed
/// (local/test mode).
///
/// The execute phase already handles per-tool-call checkpointing via
/// `CheckpointService::save` per `LoopConfig::checkpoint_every_n_tool_calls`.
/// This hook is for the loop-level checkpoint that captures the full iteration
/// summary (goal + step history + decide output) for run resumability.
#[async_trait::async_trait]
pub trait CheckpointHook: Send + Sync {
    /// Persist a snapshot of the current orchestration iteration.
    ///
    /// Called unconditionally after execute — implementations may apply their
    /// own skip logic (e.g., skip if 0 tool calls were dispatched this step).
    async fn save(
        &self,
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
        decide: &DecideOutput,
        execute: &ExecuteOutcome,
    ) -> Result<(), OrchestratorError>;
}

/// A no-op `CheckpointHook` — used in tests and local mode where durability
/// is provided by InMemoryStore rather than an external checkpoint store.
pub struct NoOpCheckpointHook;

#[async_trait::async_trait]
impl CheckpointHook for NoOpCheckpointHook {
    async fn save(
        &self,
        _ctx: &OrchestrationContext,
        _gather: &GatherOutput,
        _decide: &DecideOutput,
        _execute: &ExecuteOutcome,
    ) -> Result<(), OrchestratorError> {
        Ok(())
    }
}

// ── OrchestratorLoop ──────────────────────────────────────────────────────────

/// Drives the GATHER → DECIDE → EXECUTE loop for a single run.
///
/// # Type parameters
/// - `G`: [`GatherPhase`] implementation
/// - `D`: [`DecidePhase`] implementation
/// - `E`: [`ExecutePhase`] implementation
///
/// # Loop contract (per RFC 005)
/// - One `OrchestratorLoop` owns exactly one run's execution.
/// - All state changes flow through runtime events (RFC 002).
/// - The loop may be suspended and resumed (approval / subagent wait).
/// - `max_iterations` guards against infinite loops.
/// - `timeout_ms` provides a wall-clock deadline for the whole run.
pub struct OrchestratorLoop<G, D, E> {
    gather:  G,
    decide:  D,
    execute: E,
    config:  LoopConfig,
    checkpoint_hook: Arc<dyn CheckpointHook>,
}

impl<G, D, E> OrchestratorLoop<G, D, E>
where
    G: GatherPhase,
    D: DecidePhase,
    E: ExecutePhase,
{
    /// Construct a new loop with the given phases and configuration.
    /// Uses `NoOpCheckpointHook` — call `with_checkpoint_hook` to override.
    pub fn new(gather: G, decide: D, execute: E, config: LoopConfig) -> Self {
        Self {
            gather,
            decide,
            execute,
            config,
            checkpoint_hook: Arc::new(NoOpCheckpointHook),
        }
    }

    /// Replace the checkpoint hook (e.g., a durable Postgres checkpoint writer).
    pub fn with_checkpoint_hook(mut self, hook: Arc<dyn CheckpointHook>) -> Self {
        self.checkpoint_hook = hook;
        self
    }

    /// Drive the GATHER → DECIDE → EXECUTE cycle until a terminal state.
    ///
    /// # Returns
    /// - `Ok(LoopTermination)` for all expected stop conditions.
    /// - `Err(OrchestratorError)` only for unexpected infrastructure errors.
    ///
    /// # Resume after suspension
    /// Rebuild `OrchestrationContext` from the last checkpoint (restoring
    /// `iteration` and any relevant state), then call `run()` again.
    /// The gather phase sees the current durable state; the loop resumes.
    pub async fn run(
        &self,
        mut ctx: OrchestrationContext,
    ) -> Result<LoopTermination, OrchestratorError> {
        let deadline_ms = ctx.run_started_at_ms
            .saturating_add(self.config.timeout_ms);

        // Local step history — carried across iterations within this invocation.
        // On resume from a checkpoint the gather phase rebuilds history from the
        // store; this vec accumulates steps taken during the *current* invocation.
        let mut step_history: Vec<StepSummary> = Vec::new();

        tracing::info!(
            run_id    = %ctx.run_id,
            goal      = %ctx.goal,
            agent     = %ctx.agent_type,
            max_iter  = self.config.max_iterations,
            timeout_s = self.config.timeout_ms / 1_000,
            "orchestrator loop starting"
        );

        for _iter in 0..self.config.max_iterations {
            // ── (1) Timeout check ─────────────────────────────────────────────
            let now_ms = now_millis();
            if now_ms >= deadline_ms {
                tracing::warn!(
                    run_id    = %ctx.run_id,
                    iteration = ctx.iteration,
                    "orchestrator loop timed out"
                );
                return Ok(LoopTermination::TimedOut);
            }

            let remaining_ms = deadline_ms.saturating_sub(now_ms);

            tracing::debug!(
                run_id       = %ctx.run_id,
                iteration    = ctx.iteration,
                remaining_ms = remaining_ms,
                "iteration start"
            );

            // ── (2) GATHER ────────────────────────────────────────────────────
            let gather_output = self.gather.gather(&ctx).await.map_err(|e| {
                tracing::error!(run_id = %ctx.run_id, iteration = ctx.iteration, error = %e, "gather failed");
                e
            })?;

            tracing::debug!(
                run_id        = %ctx.run_id,
                iteration     = ctx.iteration,
                memory_chunks = gather_output.memory_chunks.len(),
                recent_events = gather_output.recent_events.len(),
                "gather complete"
            );

            // ── (3) DECIDE ────────────────────────────────────────────────────
            let decide_output = self.decide.decide(&ctx, &gather_output).await.map_err(|e| {
                tracing::error!(run_id = %ctx.run_id, iteration = ctx.iteration, error = %e, "decide failed");
                e
            })?;

            let first_action = decide_output.proposals.first()
                .map(|p| format!("{:?}", p.action_type))
                .unwrap_or_else(|| "none".to_owned());

            tracing::debug!(
                run_id     = %ctx.run_id,
                iteration  = ctx.iteration,
                proposals  = decide_output.proposals.len(),
                first      = %first_action,
                confidence = decide_output.calibrated_confidence,
                "decide complete"
            );

            // ── (4) Approval pre-check ────────────────────────────────────────
            // When requires_approval is true, the execute phase emits an
            // ApprovalRequested event and transitions the run to waiting_approval.
            // The loop suspends here — it will resume once the approval resolves.
            if decide_output.requires_approval {
                tracing::info!(
                    run_id    = %ctx.run_id,
                    iteration = ctx.iteration,
                    "decision requires approval — suspending for ApprovalRequested"
                );

                let outcome = self.execute.execute(&ctx, &decide_output).await.map_err(|e| {
                    tracing::error!(run_id = %ctx.run_id, error = %e, "execute (approval gate) failed");
                    e
                })?;

                // The execute phase returns AwaitingApproval for the relevant action.
                for result in &outcome.results {
                    if let ActionStatus::AwaitingApproval { approval_id } = &result.status {
                        tracing::info!(
                            run_id      = %ctx.run_id,
                            approval_id = %approval_id,
                            "run suspended — waiting for approval"
                        );
                        return Ok(LoopTermination::WaitingApproval {
                            approval_id: approval_id.clone(),
                        });
                    }
                }

                // Execute returned without setting AwaitingApproval — unexpected.
                return Err(OrchestratorError::Execute(
                    "requires_approval=true but execute returned no AwaitingApproval status"
                        .to_owned(),
                ));
            }

            // ── (5) EXECUTE ───────────────────────────────────────────────────
            let execute_outcome = self.execute.execute(&ctx, &decide_output).await.map_err(|e| {
                tracing::error!(run_id = %ctx.run_id, iteration = ctx.iteration, error = %e, "execute failed");
                e
            })?;

            let succeeded_count = execute_outcome.results.iter()
                .filter(|r| r.status == ActionStatus::Succeeded)
                .count();
            let failed_count = execute_outcome.results.iter()
                .filter(|r| matches!(r.status, ActionStatus::Failed { .. }))
                .count();

            tracing::debug!(
                run_id    = %ctx.run_id,
                iteration = ctx.iteration,
                succeeded = succeeded_count,
                failed    = failed_count,
                signal    = ?execute_outcome.loop_signal,
                "execute complete"
            );

            // ── (6) CHECKPOINT ────────────────────────────────────────────────
            // Build a step summary for this iteration so the gather phase can
            // reconstruct history on the next run or after a resume.
            // The execute phase has already handled per-tool-call checkpointing
            // (per LoopConfig::checkpoint_every_n_tool_calls); this step captures
            // the iteration-level summary and calls the injected checkpoint hook.
            let step_summary = build_step_summary(&ctx, &decide_output, &execute_outcome);
            step_history.push(step_summary);

            if let Err(e) = self.checkpoint_hook
                .save(&ctx, &gather_output, &decide_output, &execute_outcome)
                .await
            {
                // Checkpoint failures are logged but do NOT abort the run.
                // The next successful checkpoint will capture the current state.
                tracing::warn!(
                    run_id    = %ctx.run_id,
                    iteration = ctx.iteration,
                    error     = %e,
                    "checkpoint save failed — continuing without checkpoint"
                );
            } else {
                tracing::debug!(
                    run_id    = %ctx.run_id,
                    iteration = ctx.iteration,
                    "checkpoint saved"
                );
            }

            // ── (7) Loop signal ───────────────────────────────────────────────
            match execute_outcome.loop_signal {
                LoopSignal::Done => {
                    let summary = decide_output.proposals.iter()
                        .find(|p| p.action_type == cairn_domain::ActionType::CompleteRun)
                        .map(|p| p.description.clone())
                        .unwrap_or_else(|| "run completed".to_owned());

                    tracing::info!(
                        run_id    = %ctx.run_id,
                        iteration = ctx.iteration,
                        summary   = %summary,
                        "orchestrator loop completed"
                    );
                    return Ok(LoopTermination::Completed { summary });
                }

                LoopSignal::Failed { reason } => {
                    tracing::warn!(
                        run_id    = %ctx.run_id,
                        iteration = ctx.iteration,
                        reason    = %reason,
                        "orchestrator loop failed"
                    );
                    return Ok(LoopTermination::Failed { reason });
                }

                LoopSignal::WaitApproval { approval_id } => {
                    tracing::info!(
                        run_id      = %ctx.run_id,
                        approval_id = %approval_id,
                        "orchestrator loop suspended — waiting for approval"
                    );
                    return Ok(LoopTermination::WaitingApproval { approval_id });
                }

                LoopSignal::WaitSubagent { child_task_id } => {
                    tracing::info!(
                        run_id        = %ctx.run_id,
                        child_task_id = %child_task_id,
                        "orchestrator loop suspended — waiting for subagent"
                    );
                    return Ok(LoopTermination::WaitingSubagent { child_task_id });
                }

                LoopSignal::Continue => {
                    // Extract any tools discovered via tool_search this iteration
                    // and carry them into the next iteration's context so that
                    // LlmDecidePhase can inject their descriptors into the prompt.
                    let newly_discovered = extract_tool_search_discoveries(&execute_outcome);
                    if !newly_discovered.is_empty() {
                        tracing::debug!(
                            run_id    = %ctx.run_id,
                            iteration = ctx.iteration,
                            tools     = ?newly_discovered,
                            "tool_search discovered new tools — injecting into next prompt"
                        );
                        for name in newly_discovered {
                            if !ctx.discovered_tool_names.contains(&name) {
                                ctx.discovered_tool_names.push(name);
                            }
                        }
                    }
                    ctx.iteration = ctx.iteration.saturating_add(1);
                    tracing::debug!(
                        run_id    = %ctx.run_id,
                        iteration = ctx.iteration,
                        "continue to next iteration"
                    );
                }
            }
        }

        // All iterations exhausted.
        tracing::warn!(
            run_id     = %ctx.run_id,
            iterations = self.config.max_iterations,
            "orchestrator loop reached iteration cap"
        );
        Ok(LoopTermination::MaxIterationsReached)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extract tool names from any `tool_search` results in the execute outcome.
///
/// When the LLM calls `tool_search`, the execute phase stores the JSON result
/// in `ActionResult::tool_output`.  This function parses the `matches` array
/// and returns the discovered tool names so the loop runner can carry them into
/// the next iteration's `OrchestrationContext::discovered_tool_names`.
fn extract_tool_search_discoveries(outcome: &ExecuteOutcome) -> Vec<String> {
    let mut names = Vec::new();
    for result in &outcome.results {
        // Only look at results for tool_search invocations
        let is_tool_search = result.proposal.tool_name.as_deref() == Some("tool_search");
        if !is_tool_search {
            continue;
        }
        if let Some(output) = &result.tool_output {
            if let Some(matches) = output.get("matches").and_then(|m| m.as_array()) {
                for m in matches {
                    if let Some(name) = m.get("name").and_then(|n| n.as_str()) {
                        names.push(name.to_owned());
                    }
                }
            }
        }
    }
    names
}

/// Build a `StepSummary` from the completed iteration.
fn build_step_summary(
    ctx: &OrchestrationContext,
    decide: &DecideOutput,
    execute: &ExecuteOutcome,
) -> StepSummary {
    let action_kind = decide.proposals.first()
        .map(|p| serde_json::to_value(&p.action_type).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "unknown".to_owned()))
        .unwrap_or_else(|| "no_op".to_owned());

    let summary = decide.proposals.first()
        .map(|p| p.description.clone())
        .unwrap_or_else(|| format!("iteration {} complete", ctx.iteration));

    let succeeded = !matches!(execute.loop_signal, LoopSignal::Failed { .. });

    StepSummary {
        iteration: ctx.iteration,
        action_kind,
        summary,
        succeeded,
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cairn_domain::{
        ActionProposal, ActionType, ApprovalId, ProjectKey, RunId, SessionId, TaskId,
    };
    use crate::context::{
        ActionResult, ActionStatus, DecideOutput, ExecuteOutcome, GatherOutput,
        LoopConfig, LoopSignal, OrchestrationContext,
    };
    use crate::error::OrchestratorError;

    // ── Minimal stubs ─────────────────────────────────────────────────────────

    struct FixedGather;
    #[async_trait]
    impl GatherPhase for FixedGather {
        async fn gather(&self, _ctx: &OrchestrationContext) -> Result<GatherOutput, OrchestratorError> {
            Ok(GatherOutput::default())
        }
    }

    /// A DecidePhase stub whose behaviour is configured at construction time.
    struct ScriptedDecide {
        /// Sequence of outputs to return, one per call.
        /// Cycles back to the last entry if calls exceed the vec length.
        outputs: Vec<DecideOutput>,
        call_count: std::sync::Mutex<usize>,
    }

    impl ScriptedDecide {
        fn always(output: DecideOutput) -> Self {
            Self { outputs: vec![output], call_count: std::sync::Mutex::new(0) }
        }
    }

    #[async_trait]
    impl DecidePhase for ScriptedDecide {
        async fn decide(&self, _ctx: &OrchestrationContext, _: &GatherOutput) -> Result<DecideOutput, OrchestratorError> {
            let mut n = self.call_count.lock().unwrap();
            let idx = (*n).min(self.outputs.len() - 1);
            *n += 1;
            Ok(self.outputs[idx].clone())
        }
    }

    struct ScriptedExecute {
        signal: LoopSignal,
    }

    #[async_trait]
    impl ExecutePhase for ScriptedExecute {
        async fn execute(&self, _ctx: &OrchestrationContext, decide: &DecideOutput) -> Result<ExecuteOutcome, OrchestratorError> {
            let results = decide.proposals.iter().map(|p| ActionResult {
                proposal: p.clone(),
                status: ActionStatus::Succeeded,
                tool_output: None,
                invocation_id: None,
            }).collect();
            Ok(ExecuteOutcome {
                results,
                loop_signal: self.signal.clone(),
            })
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn ctx() -> OrchestrationContext {
        OrchestrationContext {
            project:          ProjectKey::new("t", "w", "p"),
            session_id:       SessionId::new("sess"),
            run_id:           RunId::new("run"),
            task_id:          None,
            iteration:        0,
            goal:             "test goal".to_owned(),
            agent_type:       "test_agent".to_owned(),
            run_started_at_ms: now_millis(),
            discovered_tool_names: vec![],
        }
    }

    fn complete_run_proposal() -> ActionProposal {
        ActionProposal {
            action_type:      ActionType::CompleteRun,
            description:      "all done".to_owned(),
            confidence:       0.95,
            tool_name:        None,
            tool_args:        None,
            requires_approval: false,
        }
    }

    fn decide_done() -> DecideOutput {
        DecideOutput {
            raw_response: r#"[{"action_type":"complete_run"}]"#.to_owned(),
            proposals: vec![complete_run_proposal()],
            calibrated_confidence: 0.95,
            requires_approval: false,
            model_id: "test-model".to_owned(),
            latency_ms: 10,
        }
    }

    fn decide_tool(tool: &str) -> DecideOutput {
        DecideOutput {
            raw_response: format!(r#"[{{"action_type":"invoke_tool","tool_name":"{tool}"}}]"#),
            proposals: vec![ActionProposal {
                action_type:      ActionType::InvokeTool,
                description:      format!("call {tool}"),
                confidence:       0.8,
                tool_name:        Some(tool.to_owned()),
                tool_args:        Some(serde_json::json!({})),
                requires_approval: false,
            }],
            calibrated_confidence: 0.8,
            requires_approval: false,
            model_id: "test-model".to_owned(),
            latency_ms: 20,
        }
    }

    // ── (1) Timeout ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn timeout_returns_timed_out() {
        let mut past_ctx = ctx();
        past_ctx.run_started_at_ms = 0; // started at epoch = already timed out

        let config = LoopConfig { timeout_ms: 1, ..Default::default() };
        let lp = OrchestratorLoop::new(FixedGather, ScriptedDecide::always(decide_done()),
            ScriptedExecute { signal: LoopSignal::Done }, config);

        let result = lp.run(past_ctx).await.unwrap();
        assert!(matches!(result, LoopTermination::TimedOut));
    }

    // ── (2–5) Happy path: Continue × N then Done ──────────────────────────────

    #[tokio::test]
    async fn two_iterations_then_done() {
        // First two calls return Continue; third returns Done.
        let config = LoopConfig { max_iterations: 10, ..Default::default() };

        struct CountingExecute {
            calls: std::sync::Mutex<u32>,
        }
        #[async_trait]
        impl ExecutePhase for CountingExecute {
            async fn execute(&self, _ctx: &OrchestrationContext, decide: &DecideOutput) -> Result<ExecuteOutcome, OrchestratorError> {
                let mut n = self.calls.lock().unwrap();
                *n += 1;
                let signal = if *n < 3 { LoopSignal::Continue } else { LoopSignal::Done };
                let results = decide.proposals.iter().map(|p| ActionResult {
                    proposal: p.clone(),
                    status: ActionStatus::Succeeded,
                    tool_output: None,
                    invocation_id: None,
                }).collect();
                Ok(ExecuteOutcome { results, loop_signal: signal })
            }
        }

        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_done()),
            CountingExecute { calls: std::sync::Mutex::new(0) },
            config,
        );
        let result = lp.run(ctx()).await.unwrap();
        assert!(
            matches!(result, LoopTermination::Completed { .. }),
            "expected Completed, got {result:?}"
        );
    }

    // ── Max iterations ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn max_iterations_returns_max_iterations_reached() {
        let config = LoopConfig { max_iterations: 3, ..Default::default() };
        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_tool("web_search")),
            ScriptedExecute { signal: LoopSignal::Continue },
            config,
        );
        let result = lp.run(ctx()).await.unwrap();
        assert!(matches!(result, LoopTermination::MaxIterationsReached));
    }

    // ── Execute failure ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn failed_signal_returns_failed() {
        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_tool("broken_tool")),
            ScriptedExecute { signal: LoopSignal::Failed { reason: "tool error".to_owned() } },
            LoopConfig::default(),
        );
        let result = lp.run(ctx()).await.unwrap();
        assert!(matches!(result, LoopTermination::Failed { reason } if reason == "tool error"));
    }

    // ── Approval gate ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn requires_approval_suspends_immediately() {
        let appr_id = ApprovalId::new("appr_1");
        let appr_id_clone = appr_id.clone();

        struct ApprovalExecute(ApprovalId);
        #[async_trait]
        impl ExecutePhase for ApprovalExecute {
            async fn execute(&self, _ctx: &OrchestrationContext, decide: &DecideOutput) -> Result<ExecuteOutcome, OrchestratorError> {
                let results = decide.proposals.iter().map(|p| ActionResult {
                    proposal: p.clone(),
                    status: ActionStatus::AwaitingApproval { approval_id: self.0.clone() },
                    tool_output: None,
                    invocation_id: None,
                }).collect();
                Ok(ExecuteOutcome {
                    results,
                    loop_signal: LoopSignal::WaitApproval { approval_id: self.0.clone() },
                })
            }
        }

        let needs_approval_decide = DecideOutput {
            requires_approval: true,
            proposals: vec![ActionProposal {
                action_type: ActionType::EscalateToOperator,
                description: "need approval".to_owned(),
                confidence: 0.5,
                tool_name: None,
                tool_args: None,
                requires_approval: true,
            }],
            raw_response: String::new(),
            calibrated_confidence: 0.5,
            model_id: "m".to_owned(),
            latency_ms: 0,
        };

        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(needs_approval_decide),
            ApprovalExecute(appr_id_clone),
            LoopConfig::default(),
        );
        let result = lp.run(ctx()).await.unwrap();
        assert!(
            matches!(&result, LoopTermination::WaitingApproval { approval_id } if *approval_id == appr_id),
            "expected WaitingApproval, got {result:?}"
        );
    }

    // ── WaitSubagent ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn wait_subagent_signal_suspends() {
        let child_id = TaskId::new("task_child_1");
        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_tool("spawn")),
            ScriptedExecute {
                signal: LoopSignal::WaitSubagent { child_task_id: child_id.clone() },
            },
            LoopConfig::default(),
        );
        let result = lp.run(ctx()).await.unwrap();
        assert!(
            matches!(&result, LoopTermination::WaitingSubagent { child_task_id } if *child_task_id == child_id)
        );
    }

    // ── Checkpoint hook ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn checkpoint_hook_called_after_each_iteration() {
        use std::sync::atomic::{AtomicU32, Ordering};

        struct CountingHook(Arc<AtomicU32>);
        #[async_trait::async_trait]
        impl CheckpointHook for CountingHook {
            async fn save(&self, _: &OrchestrationContext, _: &GatherOutput, _: &DecideOutput, _: &ExecuteOutcome) -> Result<(), OrchestratorError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let call_count = Arc::new(AtomicU32::new(0));
        let hook = Arc::new(CountingHook(call_count.clone()));

        struct TwoThenDone(std::sync::Mutex<u32>);
        #[async_trait]
        impl ExecutePhase for TwoThenDone {
            async fn execute(&self, _: &OrchestrationContext, decide: &DecideOutput) -> Result<ExecuteOutcome, OrchestratorError> {
                let mut n = self.0.lock().unwrap();
                *n += 1;
                let signal = if *n < 3 { LoopSignal::Continue } else { LoopSignal::Done };
                let results = decide.proposals.iter().map(|p| ActionResult {
                    proposal: p.clone(), status: ActionStatus::Succeeded,
                    tool_output: None, invocation_id: None,
                }).collect();
                Ok(ExecuteOutcome { results, loop_signal: signal })
            }
        }

        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_done()),
            TwoThenDone(std::sync::Mutex::new(0)),
            LoopConfig::default(),
        )
        .with_checkpoint_hook(hook);

        let result = lp.run(ctx()).await.unwrap();
        assert!(matches!(result, LoopTermination::Completed { .. }));
        // Checkpoint hook must be called once per completed iteration (3 total).
        assert_eq!(
            call_count.load(Ordering::SeqCst), 3,
            "checkpoint hook must be called after each of the 3 iterations"
        );
    }

    // ── Step summary accumulation ─────────────────────────────────────────────

    #[tokio::test]
    async fn build_step_summary_captures_action_kind_and_success() {
        let context = ctx();
        let decide = decide_tool("search");
        let exec = ExecuteOutcome {
            results: vec![ActionResult {
                proposal: decide.proposals[0].clone(),
                status: ActionStatus::Succeeded,
                tool_output: Some(serde_json::json!({"result": "ok"})),
                invocation_id: None,
            }],
            loop_signal: LoopSignal::Continue,
        };

        let summary = build_step_summary(&context, &decide, &exec);
        assert_eq!(summary.iteration, 0);
        assert_eq!(summary.action_kind, "invoke_tool");
        assert!(summary.succeeded);
        assert!(summary.summary.contains("search"));
    }

    // ── tool_search discovery injection ──────────────────────────────────────

    /// Verify that tool_search results in `tool_output` are extracted and
    /// carried into `ctx.discovered_tool_names` for the next iteration.
    #[test]
    fn extract_tool_search_discoveries_finds_matches() {
        let outcome = ExecuteOutcome {
            results: vec![ActionResult {
                proposal: ActionProposal {
                    action_type:       ActionType::InvokeTool,
                    description:       "search for tools".to_owned(),
                    confidence:        0.8,
                    tool_name:         Some("tool_search".to_owned()),
                    tool_args:         None,
                    requires_approval: false,
                },
                status:       ActionStatus::Succeeded,
                tool_output:  Some(serde_json::json!({
                    "matches": [
                        { "name": "shell_exec",   "description": "run shell commands" },
                        { "name": "graph_query",  "description": "query the graph" },
                    ],
                    "total": 2,
                })),
                invocation_id: None,
            }],
            loop_signal: LoopSignal::Continue,
        };

        let discovered = extract_tool_search_discoveries(&outcome);
        assert_eq!(discovered.len(), 2);
        assert!(discovered.contains(&"shell_exec".to_owned()));
        assert!(discovered.contains(&"graph_query".to_owned()));
    }

    #[test]
    fn extract_tool_search_discoveries_ignores_non_tool_search() {
        let outcome = ExecuteOutcome {
            results: vec![ActionResult {
                proposal: ActionProposal {
                    action_type: ActionType::InvokeTool,
                    description: "call something else".to_owned(),
                    confidence: 0.9,
                    tool_name: Some("memory_search".to_owned()),
                    tool_args: None,
                    requires_approval: false,
                },
                status: ActionStatus::Succeeded,
                tool_output: Some(serde_json::json!({
                    "matches": [{ "name": "should_not_appear" }]
                })),
                invocation_id: None,
            }],
            loop_signal: LoopSignal::Continue,
        };

        let discovered = extract_tool_search_discoveries(&outcome);
        assert!(discovered.is_empty(), "non-tool_search results must not produce discoveries");
    }

    #[test]
    fn extract_tool_search_discoveries_empty_matches() {
        let outcome = ExecuteOutcome {
            results: vec![ActionResult {
                proposal: ActionProposal {
                    action_type: ActionType::InvokeTool,
                    description: "search".to_owned(),
                    confidence: 0.5,
                    tool_name: Some("tool_search".to_owned()),
                    tool_args: None,
                    requires_approval: false,
                },
                status: ActionStatus::Succeeded,
                tool_output: Some(serde_json::json!({ "matches": [], "total": 0 })),
                invocation_id: None,
            }],
            loop_signal: LoopSignal::Continue,
        };

        let discovered = extract_tool_search_discoveries(&outcome);
        assert!(discovered.is_empty());
    }

    /// Integration test: after a tool_search invocation, the loop carries the
    /// discovered names into ctx.discovered_tool_names for the next iteration.
    #[tokio::test]
    async fn loop_runner_carries_discovered_tools_to_next_iteration() {
        use std::sync::Mutex;

        // Capture the ctx seen at each decide() call
        struct CapturingDecide {
            captured: std::sync::Arc<Mutex<Vec<Vec<String>>>>,
            call_n:   Mutex<u32>,
        }
        #[async_trait]
        impl DecidePhase for CapturingDecide {
            async fn decide(
                &self, ctx: &OrchestrationContext, _: &GatherOutput
            ) -> Result<DecideOutput, OrchestratorError> {
                self.captured.lock().unwrap()
                    .push(ctx.discovered_tool_names.clone());
                let n = { let mut g = self.call_n.lock().unwrap(); *g += 1; *g };
                // First call: invoke tool_search; second call: done
                let (action, tool_name, tool_args) = if n == 1 {
                    (ActionType::InvokeTool, Some("tool_search".to_owned()),
                     Some(serde_json::json!({"query":"shell"})))
                } else {
                    (ActionType::CompleteRun, None, None)
                };
                Ok(DecideOutput {
                    raw_response: String::new(),
                    proposals: vec![ActionProposal {
                        action_type: action, description: "step".to_owned(),
                        confidence: 0.9, tool_name, tool_args, requires_approval: false,
                    }],
                    calibrated_confidence: 0.9,
                    requires_approval: false,
                    model_id: "test".to_owned(),
                    latency_ms: 0,
                })
            }
        }

        // Execute returns tool_search results on first call, Done on second
        struct DiscoveryExecute { calls: Mutex<u32> }
        #[async_trait]
        impl ExecutePhase for DiscoveryExecute {
            async fn execute(
                &self, _: &OrchestrationContext, decide: &DecideOutput,
            ) -> Result<ExecuteOutcome, OrchestratorError> {
                let n = { let mut g = self.calls.lock().unwrap(); *g += 1; *g };
                let (signal, tool_output) = if n == 1 {
                    (LoopSignal::Continue, Some(serde_json::json!({
                        "matches": [{"name":"shell_exec","description":"run shell"}],
                        "total": 1,
                    })))
                } else {
                    (LoopSignal::Done, None)
                };
                let results = decide.proposals.iter().map(|p| ActionResult {
                    proposal: p.clone(), status: ActionStatus::Succeeded,
                    tool_output: tool_output.clone(), invocation_id: None,
                }).collect();
                Ok(ExecuteOutcome { results, loop_signal: signal })
            }
        }

        let shared_captured: std::sync::Arc<Mutex<Vec<Vec<String>>>> =
            std::sync::Arc::new(Mutex::new(vec![]));
        let capturing = CapturingDecide {
            captured: shared_captured.clone(),
            call_n:   Mutex::new(0),
        };

        let lp = OrchestratorLoop::new(
            FixedGather, capturing,
            DiscoveryExecute { calls: Mutex::new(0) },
            LoopConfig { max_iterations: 5, ..Default::default() },
        );

        let result = lp.run(ctx()).await.unwrap();
        assert!(matches!(result, LoopTermination::Completed { .. }));

        let snapshots = shared_captured.lock().unwrap();
        // First decide: no discovered tools yet
        assert!(snapshots[0].is_empty(),
            "iteration 0 must have no discovered tools yet");
        // Second decide: shell_exec must be in discovered_tool_names
        assert!(snapshots[1].contains(&"shell_exec".to_owned()),
            "iteration 1 must see shell_exec from prior tool_search result");
    }
}
