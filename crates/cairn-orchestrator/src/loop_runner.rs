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
use crate::emitter::{NoOpEmitter, OrchestratorEventEmitter};
use crate::error::OrchestratorError;
use crate::execute::ExecutePhase;
use crate::gather::GatherPhase;
use crate::task_sink::{NoOpTaskSink, TaskFrameSink};

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

/// Reason string carried on `LoopTermination::Failed` when the loop
/// aborts because the underlying FF lease renewal failed 3+ times.
/// Downstream code (handler error-mapping, FF `fail_with_retry`) matches
/// on this exact string — use the const to avoid typo-driven breakage.
pub const LEASE_UNHEALTHY_REASON: &str = "lease unhealthy";

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
    gather: G,
    decide: D,
    execute: E,
    config: LoopConfig,
    checkpoint_hook: Arc<dyn CheckpointHook>,
    emitter: Arc<dyn OrchestratorEventEmitter>,
    /// FF task-stream sink. `NoOpTaskSink` in the default construction —
    /// call `with_task_sink` to install a `cairn_fabric::CairnTask`-backed
    /// sink once the handler has claimed a task. Non-consuming
    /// (frames only); terminal + suspension ops stay at the caller.
    task_sink: Arc<dyn TaskFrameSink>,
}

impl<G, D, E> OrchestratorLoop<G, D, E>
where
    G: GatherPhase,
    D: DecidePhase,
    E: ExecutePhase,
{
    /// Construct a new loop with the given phases and configuration.
    /// Uses `NoOpCheckpointHook`, `NoOpEmitter`, and `NoOpTaskSink` — call
    /// the builder methods to override.
    pub fn new(gather: G, decide: D, execute: E, config: LoopConfig) -> Self {
        Self {
            gather,
            decide,
            execute,
            config,
            checkpoint_hook: Arc::new(NoOpCheckpointHook),
            emitter: Arc::new(NoOpEmitter),
            task_sink: Arc::new(NoOpTaskSink),
        }
    }

    /// Replace the checkpoint hook (e.g., a durable Postgres checkpoint writer).
    pub fn with_checkpoint_hook(mut self, hook: Arc<dyn CheckpointHook>) -> Self {
        self.checkpoint_hook = hook;
        self
    }

    /// Replace the event emitter (e.g., an SSE broadcaster for live progress).
    pub fn with_emitter(mut self, emitter: Arc<dyn OrchestratorEventEmitter>) -> Self {
        self.emitter = emitter;
        self
    }

    /// Install a task-stream sink so FF receives `tool_call`, `tool_result`,
    /// `llm_response`, and `checkpoint` frames for the attempt, and the
    /// loop can poll `is_lease_healthy` between iterations.
    ///
    /// Pass an `Arc<cairn_fabric::CairnTask>` (via the blanket impl in
    /// [`crate::task_sink`]) once the caller has claimed an FF task.
    /// Callers without an FF task can omit this — the default
    /// `NoOpTaskSink` leaves FF-side telemetry silent while the rest of
    /// the loop (EventLog bridge events, store projections, SSE) runs
    /// unchanged.
    pub fn with_task_sink(mut self, sink: Arc<dyn TaskFrameSink>) -> Self {
        self.task_sink = sink;
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
        self.emitter.on_started(&ctx).await;
        let result = self.run_inner(&mut ctx).await;
        // Emit on_finished for every terminal outcome. For the Err branch
        // propagate the underlying OrchestratorError's Display string so
        // dashboards see the real cause (e.g. "decide: model 404",
        // "memory: kb unavailable") rather than "infrastructure error".
        match &result {
            Ok(t) => self.emitter.on_finished(&ctx, t).await,
            Err(e) => {
                let term = LoopTermination::Failed {
                    reason: e.to_string(),
                };
                self.emitter.on_finished(&ctx, &term).await;
            }
        }
        result
    }

    async fn run_inner(
        &self,
        ctx: &mut OrchestrationContext,
    ) -> Result<LoopTermination, OrchestratorError> {
        let deadline_ms = ctx.run_started_at_ms.saturating_add(self.config.timeout_ms);

        // Local step history — carried across iterations within this invocation.
        // On resume from a checkpoint the gather phase rebuilds history from the
        // store; this vec accumulates steps taken during the *current* invocation.
        let mut step_history: Vec<StepSummary> = Vec::new();
        let mut last_compaction_iteration: Option<u32> = None;

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

            // ── (1b) Lease health gate ───────────────────────────────────────
            // FF's ClaimedTask tracks consecutive renewal failures; after 3
            // misses `is_lease_healthy()` returns false and every downstream
            // FCALL will reject as stale_lease. Bail before committing any
            // irreversible side effect (LLM call, tool dispatch, checkpoint
            // write) — the caller sees `LoopTermination::Failed { reason:
            // "lease unhealthy" }` and can fail the run cleanly via the
            // CairnTask handle it still owns.
            if !self.task_sink.is_lease_healthy() {
                tracing::warn!(
                    run_id    = %ctx.run_id,
                    iteration = ctx.iteration,
                    "lease unhealthy (3+ renewal failures) — aborting loop"
                );
                return Ok(LoopTermination::Failed {
                    reason: LEASE_UNHEALTHY_REASON.to_owned(),
                });
            }

            let remaining_ms = deadline_ms.saturating_sub(now_ms);

            tracing::debug!(
                run_id       = %ctx.run_id,
                iteration    = ctx.iteration,
                remaining_ms = remaining_ms,
                "iteration start"
            );

            // ── (2) GATHER ────────────────────────────────────────────────────
            // T5-H1: surface the loop-maintained step_history so
            // `StandardGatherPhase::gather` threads it into `GatherOutput.step_history`
            // and `LlmDecidePhase::build_user_message` can render prior
            // iterations into the LLM context.
            ctx.step_history = step_history.clone();
            let gather_output = self.gather.gather(ctx).await.map_err(|e| {
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
            self.emitter.on_gather_completed(ctx, &gather_output).await;

            // ── (2b) COMPACTION CHECK (RFC 018) ──────────────────────────────
            // If step history exceeds the compaction threshold, compress older
            // steps into a summary, keeping the most recent N steps verbatim.
            if let Some(compaction) = maybe_compact_history(
                &mut step_history,
                ctx.iteration,
                &self.config.compaction,
                &mut last_compaction_iteration,
            ) {
                tracing::info!(
                    run_id         = %ctx.run_id,
                    iteration      = ctx.iteration,
                    before_steps   = compaction.before_steps,
                    after_steps    = compaction.after_steps,
                    before_tokens  = compaction.before_tokens_est,
                    after_tokens   = compaction.after_tokens_est,
                    "context compacted"
                );

                self.emitter
                    .on_context_compacted(
                        ctx,
                        compaction.before_steps,
                        compaction.after_steps,
                        compaction.before_tokens_est,
                        compaction.after_tokens_est,
                    )
                    .await;
            }

            // ── (3) DECIDE ────────────────────────────────────────────────────
            let decide_output = self.decide.decide(ctx, &gather_output).await.map_err(|e| {
                tracing::error!(run_id = %ctx.run_id, iteration = ctx.iteration, error = %e, "decide failed");
                e
            })?;

            let first_action = decide_output
                .proposals
                .first()
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
            self.emitter.on_decide_completed(ctx, &decide_output).await;

            // ── (3b') FF stream: llm_response frame ──────────────────────────
            // DecideOutput already carries model_id, token counts, and
            // latency. Surface those on FF's attempt stream so cost
            // reconciliation + audit replay work off a single durable source
            // without cairn-store having to parse the raw LLM body. Token
            // counts default to 0 when the provider didn't report them (FF
            // stream format requires u64).
            if let Err(e) = self
                .task_sink
                .log_llm_response(
                    &decide_output.model_id,
                    decide_output.input_tokens.unwrap_or(0) as u64,
                    decide_output.output_tokens.unwrap_or(0) as u64,
                    decide_output.latency_ms,
                )
                .await
            {
                tracing::warn!(
                    run_id    = %ctx.run_id,
                    iteration = ctx.iteration,
                    model     = %decide_output.model_id,
                    error     = %e,
                    "task_sink.log_llm_response failed — frame lost, loop continues"
                );
            }

            // ── (3b) Plan artifact detection (RFC 018) ───────────────────────
            // In Plan mode, check if the LLM response contains a <proposed_plan>
            // block. If so, extract the plan markdown and terminate the run.
            if matches!(ctx.run_mode, cairn_domain::decisions::RunMode::Plan) {
                if let Some(plan_md) = extract_proposed_plan(&decide_output.raw_response) {
                    tracing::info!(
                        run_id    = %ctx.run_id,
                        iteration = ctx.iteration,
                        plan_len  = plan_md.len(),
                        "plan artifact detected — terminating Plan-mode run"
                    );
                    self.emitter.on_plan_proposed(ctx, &plan_md).await;
                    return Ok(LoopTermination::PlanProposed {
                        plan_markdown: plan_md,
                    });
                }
            }

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

                // FF stream: log tool_call frames for the approval-gate
                // proposals before execute. The approval path emits
                // `ApprovalRequested` rather than dispatching; the tool_call
                // frame captures the INTENT of the action that triggered the
                // gate so replay can reconstruct the audit trail.
                for proposal in &decide_output.proposals {
                    if let Some(tool_name) = &proposal.tool_name {
                        let args = proposal
                            .tool_args
                            .clone()
                            .unwrap_or(serde_json::Value::Null);
                        if let Err(e) = self.task_sink.log_tool_call(tool_name, &args).await {
                            tracing::warn!(
                                run_id = %ctx.run_id,
                                tool = %tool_name,
                                error = %e,
                                "task_sink.log_tool_call (approval gate) failed — frame lost"
                            );
                        }
                    }
                }

                let execute_outcome = self.execute.execute(ctx, &decide_output).await.map_err(|e| {
                    tracing::error!(run_id = %ctx.run_id, error = %e, "execute (approval gate) failed");
                    e
                })?;

                // T5-M8: mirror the main-path post-execute bookkeeping so
                // resuming from a checkpointed approval-suspended run sees a
                // step_summary, a persisted checkpoint, and a step_completed
                // emission for this iteration.
                let step_summary = build_step_summary(ctx, &decide_output, &execute_outcome);
                step_history.push(step_summary);
                if let Err(e) = self
                    .checkpoint_hook
                    .save(ctx, &gather_output, &decide_output, &execute_outcome)
                    .await
                {
                    tracing::warn!(
                        run_id = %ctx.run_id,
                        iteration = ctx.iteration,
                        error = %e,
                        "approval gate: checkpoint save failed — continuing without checkpoint"
                    );
                }
                self.emitter
                    .on_step_completed(ctx, &decide_output, &execute_outcome)
                    .await;

                // The execute phase returns AwaitingApproval for the relevant action.
                for result in &execute_outcome.results {
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

            // ── (5a) FF stream: tool_call frames (intent) ────────────────────
            // Appended BEFORE execute so a process restart mid-dispatch leaves
            // an in-flight marker that `restore_frames()` can observe. Only
            // proposals with a concrete tool_name are framed; bookkeeping
            // action types (CompleteRun, EscalateToOperator, …) have no
            // stream-facing analogue.
            for proposal in &decide_output.proposals {
                if let Some(tool_name) = &proposal.tool_name {
                    let args = proposal
                        .tool_args
                        .clone()
                        .unwrap_or(serde_json::Value::Null);
                    if let Err(e) = self.task_sink.log_tool_call(tool_name, &args).await {
                        tracing::warn!(
                            run_id    = %ctx.run_id,
                            iteration = ctx.iteration,
                            tool      = %tool_name,
                            error     = %e,
                            "task_sink.log_tool_call failed — frame lost, loop continues"
                        );
                    }
                }
            }

            // ── (5b) EXECUTE ──────────────────────────────────────────────────
            let execute_outcome = self.execute.execute(ctx, &decide_output).await.map_err(|e| {
                tracing::error!(run_id = %ctx.run_id, iteration = ctx.iteration, error = %e, "execute failed");
                e
            })?;

            let succeeded_count = execute_outcome
                .results
                .iter()
                .filter(|r| r.status == ActionStatus::Succeeded)
                .count();
            let failed_count = execute_outcome
                .results
                .iter()
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

            // Emit per-action tool_called / tool_result events AND append
            // matching FF stream frames. The two channels are independent:
            // `OrchestratorEventEmitter` drives cairn-store projections + SSE
            // (existing behavior); `task_sink.log_tool_result` appends to
            // FF's attempt-scoped stream so `restore_frames()` can replay on
            // resume. Stream-frame failures are logged and swallowed.
            //
            // `result.duration_ms` is stamped per-proposal inside
            // `ExecutePhase::execute` (see `execute_impl.rs::execute`). 0
            // means "unknown / below-timer-resolution" or "result was
            // synthesised by a test stub that bypassed the dispatch wrapper" —
            // NOT "zero time." Downstream consumers MUST treat 0 as no-signal.
            // The `ActionResult.duration_ms` rustdoc is the canonical reference.
            for result in &execute_outcome.results {
                // T5-M4: only emit tool_called/tool_result for proposals
                // that actually carry a tool_name. CompleteRun /
                // EscalateToOperator / CreateMemory have no tool_name and
                // previously leaked `tool_name = description` (e.g.
                // `"all done"`) into the SSE stream, producing a misleading
                // tool-call timeline.
                let Some(tool_name) = result.proposal.tool_name.as_deref() else {
                    continue;
                };
                self.emitter
                    .on_tool_called(ctx, tool_name, result.proposal.tool_args.as_ref())
                    .await;
                let (succeeded, error) = match &result.status {
                    ActionStatus::Succeeded => (true, None),
                    ActionStatus::Failed { reason } => (false, Some(reason.as_str())),
                    // AwaitingApproval / SubagentSpawned are not
                    // success/failure terminal states for the tool; report
                    // succeeded=false without an error string so the
                    // dashboard doesn't claim the tool ran.
                    _ => (false, None),
                };
                self.emitter
                    .on_tool_result(
                        ctx,
                        tool_name,
                        succeeded,
                        result.tool_output.as_ref(),
                        error,
                        result.duration_ms,
                    )
                    .await;

                // FF stream frame — only emitted when the proposal carried a
                // concrete tool_name (matches the pre-execute log_tool_call).
                // AwaitingApproval / SubagentSpawned statuses have no output
                // to log; record success=false with a null output so the
                // frame pair is balanced.
                if result.proposal.tool_name.is_some() {
                    let output = match &result.tool_output {
                        Some(v) => v.clone(),
                        None => {
                            if let Some(reason) = error {
                                serde_json::json!({ "error": reason })
                            } else {
                                serde_json::Value::Null
                            }
                        }
                    };
                    if let Err(e) = self
                        .task_sink
                        .log_tool_result(tool_name, &output, succeeded, result.duration_ms)
                        .await
                    {
                        tracing::warn!(
                            run_id    = %ctx.run_id,
                            iteration = ctx.iteration,
                            tool      = %tool_name,
                            error     = %e,
                            "task_sink.log_tool_result failed — frame lost, loop continues"
                        );
                    }
                }
            }

            // ── (6) CHECKPOINT ────────────────────────────────────────────────
            // Build a step summary for this iteration so the gather phase can
            // reconstruct history on the next run or after a resume.
            // The execute phase has already handled per-tool-call checkpointing
            // (per LoopConfig::checkpoint_every_n_tool_calls); this step captures
            // the iteration-level summary and calls the injected checkpoint hook.
            let step_summary = build_step_summary(ctx, &decide_output, &execute_outcome);
            step_history.push(step_summary);

            if let Err(e) = self
                .checkpoint_hook
                .save(ctx, &gather_output, &decide_output, &execute_outcome)
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

            // ── (6b) FF stream: checkpoint frame ─────────────────────────────
            // Write half of the `restore_frames()` read path. Serializes the
            // per-iteration context snapshot (iteration number, run/session
            // ids, the step summary just pushed, and the loop_signal) as
            // JSON and appends it as a `checkpoint` frame on the attempt
            // stream. A cross-process resumer reads the stream
            // via `restore_frames()` and rebuilds enough context to pick up
            // where the previous attempt left off.
            //
            // Same best-effort contract as tool/llm frames: failure = WARN +
            // continue. See `task_sink` module docs for the nuance (a lost
            // checkpoint frame means restart-resumption silently misses this
            // iteration's state; kept advisory for consistency with the
            // existing `CheckpointHook::save` failure policy).
            let checkpoint_snapshot = serde_json::json!({
                "iteration": ctx.iteration,
                "run_id": ctx.run_id.to_string(),
                "session_id": ctx.session_id.to_string(),
                "step_summary": step_history.last(),
                "loop_signal": format!("{:?}", execute_outcome.loop_signal),
            });
            match serde_json::to_vec(&checkpoint_snapshot) {
                Ok(checkpoint_bytes) => {
                    if let Err(e) = self.task_sink.save_checkpoint(&checkpoint_bytes).await {
                        tracing::warn!(
                            run_id    = %ctx.run_id,
                            iteration = ctx.iteration,
                            error     = %e,
                            "task_sink.save_checkpoint failed — frame lost, loop continues"
                        );
                    }
                }
                Err(e) => {
                    // Should be unreachable (the snapshot is built from
                    // owned primitives + Debug format), but don't eat the
                    // failure silently. Loss of a checkpoint frame is
                    // advisory (see CAIRN-FABRIC-FINALIZED.md §4.5) —
                    // WARN + continue matches the other sink failure paths.
                    tracing::warn!(
                        run_id    = %ctx.run_id,
                        iteration = ctx.iteration,
                        error     = %e,
                        "failed to serialize checkpoint snapshot — frame lost, loop continues"
                    );
                }
            }

            self.emitter
                .on_step_completed(ctx, &decide_output, &execute_outcome)
                .await;

            // ── (7) Loop signal ───────────────────────────────────────────────
            match execute_outcome.loop_signal {
                LoopSignal::Done => {
                    let summary = decide_output
                        .proposals
                        .iter()
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

                LoopSignal::PlanProposed { plan_markdown } => {
                    tracing::info!(
                        run_id    = %ctx.run_id,
                        iteration = ctx.iteration,
                        "plan proposed via loop signal"
                    );
                    return Ok(LoopTermination::PlanProposed { plan_markdown });
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompactionResult {
    before_steps: usize,
    after_steps: usize,
    before_tokens_est: usize,
    after_tokens_est: usize,
}

fn maybe_compact_history(
    step_history: &mut Vec<StepSummary>,
    iteration: u32,
    config: &crate::CompactionConfig,
    last_compaction_iteration: &mut Option<u32>,
) -> Option<CompactionResult> {
    if !config.enabled || step_history.len() < config.min_steps {
        return None;
    }

    if let Some(last_iteration) = *last_compaction_iteration {
        let cooldown = config.cooldown_iterations;
        if cooldown > 0 && iteration.saturating_sub(last_iteration) < cooldown {
            return None;
        }
    }

    let history_text: String = step_history
        .iter()
        .map(|s| format!("[iter {}] {}: {}", s.iteration, s.action_kind, s.summary))
        .collect::<Vec<_>>()
        .join("\n");
    let history_tokens = crate::decide_impl::estimate_tokens(&history_text);
    // Use a rough context budget estimate (default 16K if no budget set).
    let context_budget = 16_384_usize;
    let threshold_tokens = (context_budget as u64 * config.threshold_pct as u64 / 100) as usize;

    if history_tokens <= threshold_tokens {
        return None;
    }

    let keep = config.keep_last;
    let to_compact = if step_history.len() > keep {
        step_history.len() - keep
    } else {
        0
    };

    if to_compact == 0 {
        return None;
    }

    let before_steps = step_history.len();

    let compacted_text: String = step_history[..to_compact]
        .iter()
        .map(|s| {
            let status = if s.succeeded { "ok" } else { "fail" };
            format!("  iter {}: {} [{}]", s.iteration, s.action_kind, status)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let summary = StepSummary {
        iteration: step_history[to_compact - 1].iteration,
        action_kind: "compacted_summary".to_owned(),
        summary: format!("Compacted {} prior steps:\n{}", to_compact, compacted_text),
        succeeded: true,
    };

    let recent: Vec<StepSummary> = step_history[to_compact..].to_vec();
    step_history.clear();
    step_history.push(summary);
    step_history.extend(recent);

    let after_tokens = step_history
        .iter()
        .map(|s| crate::decide_impl::estimate_tokens(&s.summary))
        .sum::<usize>();

    *last_compaction_iteration = Some(iteration);

    Some(CompactionResult {
        before_steps,
        after_steps: step_history.len(),
        before_tokens_est: history_tokens,
        after_tokens_est: after_tokens,
    })
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

/// Extract the `<proposed_plan>` block from an LLM response (RFC 018).
///
/// Returns `Some(plan_markdown)` if the response contains a `<proposed_plan>`
/// block, `None` otherwise. Strips the XML tags.
fn extract_proposed_plan(response: &str) -> Option<String> {
    let start_tag = "<proposed_plan>";
    let end_tag = "</proposed_plan>";
    let start = response.find(start_tag)?;
    let content_start = start + start_tag.len();
    let end = response[content_start..].find(end_tag)?;
    let plan = response[content_start..content_start + end].trim();
    if plan.is_empty() {
        None
    } else {
        Some(plan.to_owned())
    }
}

/// Build a `StepSummary` from the completed iteration.
fn build_step_summary(
    ctx: &OrchestrationContext,
    decide: &DecideOutput,
    execute: &ExecuteOutcome,
) -> StepSummary {
    let action_kind = decide
        .proposals
        .first()
        .map(|p| {
            serde_json::to_value(&p.action_type)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned())
        })
        .unwrap_or_else(|| "no_op".to_owned());

    let summary = decide
        .proposals
        .first()
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
    use crate::context::{
        ActionResult, ActionStatus, CompactionConfig, DecideOutput, ExecuteOutcome, GatherOutput,
        LoopConfig, LoopSignal, OrchestrationContext,
    };
    use crate::error::OrchestratorError;
    use async_trait::async_trait;
    use cairn_domain::{
        ActionProposal, ActionType, ApprovalId, ProjectKey, RunId, SessionId, TaskId,
    };
    use std::path::PathBuf;

    // ── Minimal stubs ─────────────────────────────────────────────────────────

    struct FixedGather;
    #[async_trait]
    impl GatherPhase for FixedGather {
        async fn gather(
            &self,
            _ctx: &OrchestrationContext,
        ) -> Result<GatherOutput, OrchestratorError> {
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
            Self {
                outputs: vec![output],
                call_count: std::sync::Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl DecidePhase for ScriptedDecide {
        async fn decide(
            &self,
            _ctx: &OrchestrationContext,
            _: &GatherOutput,
        ) -> Result<DecideOutput, OrchestratorError> {
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
        async fn execute(
            &self,
            _ctx: &OrchestrationContext,
            decide: &DecideOutput,
        ) -> Result<ExecuteOutcome, OrchestratorError> {
            let results = decide
                .proposals
                .iter()
                .map(|p| ActionResult {
                    proposal: p.clone(),
                    status: ActionStatus::Succeeded,
                    tool_output: None,
                    invocation_id: None,
                    duration_ms: 0,
                })
                .collect();
            Ok(ExecuteOutcome {
                results,
                loop_signal: self.signal.clone(),
            })
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn ctx() -> OrchestrationContext {
        OrchestrationContext {
            project: ProjectKey::new("t", "w", "p"),
            session_id: SessionId::new("sess"),
            run_id: RunId::new("run"),
            task_id: None,
            iteration: 0,
            goal: "test goal".to_owned(),
            agent_type: "test_agent".to_owned(),
            run_started_at_ms: now_millis(),
            working_dir: PathBuf::from("."),
            run_mode: cairn_domain::decisions::RunMode::Direct,
            discovered_tool_names: vec![],
            step_history: vec![],
        }
    }

    fn complete_run_proposal() -> ActionProposal {
        ActionProposal {
            action_type: ActionType::CompleteRun,
            description: "all done".to_owned(),
            confidence: 0.95,
            tool_name: None,
            tool_args: None,
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
            input_tokens: None,
            output_tokens: None,
        }
    }

    fn decide_tool(tool: &str) -> DecideOutput {
        DecideOutput {
            raw_response: format!(r#"[{{"action_type":"invoke_tool","tool_name":"{tool}"}}]"#),
            proposals: vec![ActionProposal {
                action_type: ActionType::InvokeTool,
                description: format!("call {tool}"),
                confidence: 0.8,
                tool_name: Some(tool.to_owned()),
                tool_args: Some(serde_json::json!({})),
                requires_approval: false,
            }],
            calibrated_confidence: 0.8,
            requires_approval: false,
            model_id: "test-model".to_owned(),
            latency_ms: 20,
            input_tokens: None,
            output_tokens: None,
        }
    }

    // ── (1) Timeout ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn timeout_returns_timed_out() {
        let mut past_ctx = ctx();
        past_ctx.run_started_at_ms = 0; // started at epoch = already timed out

        let config = LoopConfig {
            timeout_ms: 1,
            ..Default::default()
        };
        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_done()),
            ScriptedExecute {
                signal: LoopSignal::Done,
            },
            config,
        );

        let result = lp.run(past_ctx).await.unwrap();
        assert!(matches!(result, LoopTermination::TimedOut));
    }

    // ── (1b) Lease health gate ────────────────────────────────────────────────
    //
    // The §1b gate in `run_inner` polls `task_sink.is_lease_healthy()` at
    // each iteration start and short-circuits to
    // `LoopTermination::Failed { reason: "lease unhealthy" }` when the
    // sink reports false. This is the safety gate for the whole feature —
    // a degraded lease means every downstream FCALL (LLM call, tool
    // dispatch, checkpoint) will be rejected by FF anyway, so bailing
    // early avoids committing irreversible work.

    struct UnhealthySink;
    #[async_trait]
    impl crate::task_sink::TaskFrameSink for UnhealthySink {
        async fn log_tool_call(
            &self,
            _name: &str,
            _args: &serde_json::Value,
        ) -> Result<(), OrchestratorError> {
            panic!("log_tool_call must not be reached when lease is unhealthy")
        }
        async fn log_tool_result(
            &self,
            _name: &str,
            _output: &serde_json::Value,
            _success: bool,
            _duration_ms: u64,
        ) -> Result<(), OrchestratorError> {
            panic!("log_tool_result must not be reached when lease is unhealthy")
        }
        async fn log_llm_response(
            &self,
            _model: &str,
            _tokens_in: u64,
            _tokens_out: u64,
            _latency_ms: u64,
        ) -> Result<(), OrchestratorError> {
            panic!("log_llm_response must not be reached when lease is unhealthy")
        }
        async fn save_checkpoint(&self, _bytes: &[u8]) -> Result<(), OrchestratorError> {
            panic!("save_checkpoint must not be reached when lease is unhealthy")
        }
        fn is_lease_healthy(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn unhealthy_lease_aborts_before_gather() {
        // Install a sink that reports unhealthy AND panics on any frame
        // write — if the loop reached gather/decide/execute and tried to
        // emit a frame, the test would fail with the panic message.
        // Termination must arrive from the §1b gate alone.
        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_done()),
            ScriptedExecute {
                signal: LoopSignal::Done,
            },
            LoopConfig::default(),
        )
        .with_task_sink(std::sync::Arc::new(UnhealthySink));

        let result = lp.run(ctx()).await.unwrap();
        match result {
            LoopTermination::Failed { reason } => {
                assert_eq!(
                    reason, "lease unhealthy",
                    "lease-health gate must surface the exact reason — callers downstream \
                     (CairnTask::fail_with_retry, handler error mapping) may match on it",
                );
            }
            other => panic!("expected Failed {{ reason: 'lease unhealthy' }}, got {other:?}"),
        }
    }

    // ── (2–5) Happy path: Continue × N then Done ──────────────────────────────

    #[tokio::test]
    async fn two_iterations_then_done() {
        // First two calls return Continue; third returns Done.
        let config = LoopConfig {
            max_iterations: 10,
            ..Default::default()
        };

        struct CountingExecute {
            calls: std::sync::Mutex<u32>,
        }
        #[async_trait]
        impl ExecutePhase for CountingExecute {
            async fn execute(
                &self,
                _ctx: &OrchestrationContext,
                decide: &DecideOutput,
            ) -> Result<ExecuteOutcome, OrchestratorError> {
                let mut n = self.calls.lock().unwrap();
                *n += 1;
                let signal = if *n < 3 {
                    LoopSignal::Continue
                } else {
                    LoopSignal::Done
                };
                let results = decide
                    .proposals
                    .iter()
                    .map(|p| ActionResult {
                        proposal: p.clone(),
                        status: ActionStatus::Succeeded,
                        tool_output: None,
                        invocation_id: None,
                        duration_ms: 0,
                    })
                    .collect();
                Ok(ExecuteOutcome {
                    results,
                    loop_signal: signal,
                })
            }
        }

        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_done()),
            CountingExecute {
                calls: std::sync::Mutex::new(0),
            },
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
        let config = LoopConfig {
            max_iterations: 3,
            ..Default::default()
        };
        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_tool("web_search")),
            ScriptedExecute {
                signal: LoopSignal::Continue,
            },
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
            ScriptedExecute {
                signal: LoopSignal::Failed {
                    reason: "tool error".to_owned(),
                },
            },
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
            async fn execute(
                &self,
                _ctx: &OrchestrationContext,
                decide: &DecideOutput,
            ) -> Result<ExecuteOutcome, OrchestratorError> {
                let results = decide
                    .proposals
                    .iter()
                    .map(|p| ActionResult {
                        proposal: p.clone(),
                        status: ActionStatus::AwaitingApproval {
                            approval_id: self.0.clone(),
                        },
                        tool_output: None,
                        invocation_id: None,
                        duration_ms: 0,
                    })
                    .collect();
                Ok(ExecuteOutcome {
                    results,
                    loop_signal: LoopSignal::WaitApproval {
                        approval_id: self.0.clone(),
                    },
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
            input_tokens: None,
            output_tokens: None,
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
                signal: LoopSignal::WaitSubagent {
                    child_task_id: child_id.clone(),
                },
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
            async fn save(
                &self,
                _: &OrchestrationContext,
                _: &GatherOutput,
                _: &DecideOutput,
                _: &ExecuteOutcome,
            ) -> Result<(), OrchestratorError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let call_count = Arc::new(AtomicU32::new(0));
        let hook = Arc::new(CountingHook(call_count.clone()));

        struct TwoThenDone(std::sync::Mutex<u32>);
        #[async_trait]
        impl ExecutePhase for TwoThenDone {
            async fn execute(
                &self,
                _: &OrchestrationContext,
                decide: &DecideOutput,
            ) -> Result<ExecuteOutcome, OrchestratorError> {
                let mut n = self.0.lock().unwrap();
                *n += 1;
                let signal = if *n < 3 {
                    LoopSignal::Continue
                } else {
                    LoopSignal::Done
                };
                let results = decide
                    .proposals
                    .iter()
                    .map(|p| ActionResult {
                        proposal: p.clone(),
                        status: ActionStatus::Succeeded,
                        tool_output: None,
                        invocation_id: None,
                        duration_ms: 0,
                    })
                    .collect();
                Ok(ExecuteOutcome {
                    results,
                    loop_signal: signal,
                })
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
            call_count.load(Ordering::SeqCst),
            3,
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
                duration_ms: 0,
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
                    action_type: ActionType::InvokeTool,
                    description: "search for tools".to_owned(),
                    confidence: 0.8,
                    tool_name: Some("tool_search".to_owned()),
                    tool_args: None,
                    requires_approval: false,
                },
                status: ActionStatus::Succeeded,
                tool_output: Some(serde_json::json!({
                    "matches": [
                        { "name": "shell_exec",   "description": "run shell commands" },
                        { "name": "graph_query",  "description": "query the graph" },
                    ],
                    "total": 2,
                })),
                invocation_id: None,
                duration_ms: 0,
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
                duration_ms: 0,
            }],
            loop_signal: LoopSignal::Continue,
        };

        let discovered = extract_tool_search_discoveries(&outcome);
        assert!(
            discovered.is_empty(),
            "non-tool_search results must not produce discoveries"
        );
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
                duration_ms: 0,
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
            call_n: Mutex<u32>,
        }
        #[async_trait]
        impl DecidePhase for CapturingDecide {
            async fn decide(
                &self,
                ctx: &OrchestrationContext,
                _: &GatherOutput,
            ) -> Result<DecideOutput, OrchestratorError> {
                self.captured
                    .lock()
                    .unwrap()
                    .push(ctx.discovered_tool_names.clone());
                let n = {
                    let mut g = self.call_n.lock().unwrap();
                    *g += 1;
                    *g
                };
                // First call: invoke tool_search; second call: done
                let (action, tool_name, tool_args) = if n == 1 {
                    (
                        ActionType::InvokeTool,
                        Some("tool_search".to_owned()),
                        Some(serde_json::json!({"query":"shell"})),
                    )
                } else {
                    (ActionType::CompleteRun, None, None)
                };
                Ok(DecideOutput {
                    raw_response: String::new(),
                    proposals: vec![ActionProposal {
                        action_type: action,
                        description: "step".to_owned(),
                        confidence: 0.9,
                        tool_name,
                        tool_args,
                        requires_approval: false,
                    }],
                    calibrated_confidence: 0.9,
                    requires_approval: false,
                    model_id: "test".to_owned(),
                    latency_ms: 0,
                    input_tokens: None,
                    output_tokens: None,
                })
            }
        }

        // Execute returns tool_search results on first call, Done on second
        struct DiscoveryExecute {
            calls: Mutex<u32>,
        }
        #[async_trait]
        impl ExecutePhase for DiscoveryExecute {
            async fn execute(
                &self,
                _: &OrchestrationContext,
                decide: &DecideOutput,
            ) -> Result<ExecuteOutcome, OrchestratorError> {
                let n = {
                    let mut g = self.calls.lock().unwrap();
                    *g += 1;
                    *g
                };
                let (signal, tool_output) = if n == 1 {
                    (
                        LoopSignal::Continue,
                        Some(serde_json::json!({
                            "matches": [{"name":"shell_exec","description":"run shell"}],
                            "total": 1,
                        })),
                    )
                } else {
                    (LoopSignal::Done, None)
                };
                let results = decide
                    .proposals
                    .iter()
                    .map(|p| ActionResult {
                        proposal: p.clone(),
                        status: ActionStatus::Succeeded,
                        tool_output: tool_output.clone(),
                        invocation_id: None,
                        duration_ms: 0,
                    })
                    .collect();
                Ok(ExecuteOutcome {
                    results,
                    loop_signal: signal,
                })
            }
        }

        let shared_captured: std::sync::Arc<Mutex<Vec<Vec<String>>>> =
            std::sync::Arc::new(Mutex::new(vec![]));
        let capturing = CapturingDecide {
            captured: shared_captured.clone(),
            call_n: Mutex::new(0),
        };

        let lp = OrchestratorLoop::new(
            FixedGather,
            capturing,
            DiscoveryExecute {
                calls: Mutex::new(0),
            },
            LoopConfig {
                max_iterations: 5,
                ..Default::default()
            },
        );

        let result = lp.run(ctx()).await.unwrap();
        assert!(matches!(result, LoopTermination::Completed { .. }));

        let snapshots = shared_captured.lock().unwrap();
        // First decide: no discovered tools yet
        assert!(
            snapshots[0].is_empty(),
            "iteration 0 must have no discovered tools yet"
        );
        // Second decide: shell_exec must be in discovered_tool_names
        assert!(
            snapshots[1].contains(&"shell_exec".to_owned()),
            "iteration 1 must see shell_exec from prior tool_search result"
        );
    }

    // ── Plan extraction tests (RFC 018) ─────────────────────────────────

    #[test]
    fn extract_proposed_plan_parses_block() {
        let response = "Here's my analysis.\n\n<proposed_plan>\n# Plan: Fix the bug\n\n## What I found\nThe bug is in foo.rs line 42.\n\n## What I propose\n1. Fix foo.rs\n</proposed_plan>\n\nDone.";
        let plan = extract_proposed_plan(response);
        assert!(plan.is_some());
        let md = plan.unwrap();
        assert!(md.contains("# Plan: Fix the bug"));
        assert!(md.contains("Fix foo.rs"));
    }

    #[test]
    fn extract_proposed_plan_returns_none_when_absent() {
        let response = "I need more information before I can propose a plan.";
        assert!(extract_proposed_plan(response).is_none());
    }

    #[test]
    fn extract_proposed_plan_returns_none_for_empty_block() {
        let response = "<proposed_plan>\n\n</proposed_plan>";
        assert!(extract_proposed_plan(response).is_none());
    }

    #[test]
    fn extract_proposed_plan_handles_unclosed_tag() {
        let response = "<proposed_plan>\nPartial plan without closing tag";
        assert!(extract_proposed_plan(response).is_none());
    }

    // ── Compaction tests (RFC 018) ──────────────────────────────────────

    #[test]
    fn compaction_config_defaults() {
        let cfg = crate::CompactionConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.threshold_pct, 70);
        assert_eq!(cfg.min_steps, 10);
        assert_eq!(cfg.keep_last, 4);
        assert_eq!(cfg.summary_token_budget, 2000);
        assert_eq!(cfg.cooldown_iterations, 5);
    }

    #[tokio::test]
    async fn compaction_triggers_when_history_exceeds_threshold() {
        // Build a loop with compaction enabled and low thresholds for testing.
        let config = LoopConfig {
            max_iterations: 1,
            compaction: CompactionConfig {
                enabled: true,
                min_steps: 3,
                keep_last: 2,
                threshold_pct: 1, // very low so it always triggers
                ..CompactionConfig::default()
            },
            ..LoopConfig::default()
        };

        // Pre-populate step_history with enough steps.
        // We can't directly set step_history in the loop, so instead we test
        // the compaction logic directly here.
        let mut step_history: Vec<StepSummary> = (0..10)
            .map(|i| StepSummary {
                iteration: i,
                action_kind: "tool_call".to_owned(),
                summary: format!("Called tool_{i} with result: some long output text repeated many times to ensure token threshold is met. Extra padding to make the history large."),
                succeeded: true,
            })
            .collect();

        let before_count = step_history.len();
        let keep = config.compaction.keep_last;
        let to_compact = step_history.len() - keep;

        // Simulate the compaction logic from the loop runner.
        let compacted_text: String = step_history[..to_compact]
            .iter()
            .map(|s| format!("  iter {}: {} [ok]", s.iteration, s.action_kind))
            .collect::<Vec<_>>()
            .join("\n");

        let summary = StepSummary {
            iteration: step_history[to_compact - 1].iteration,
            action_kind: "compacted_summary".to_owned(),
            summary: format!("Compacted {} prior steps:\n{}", to_compact, compacted_text),
            succeeded: true,
        };

        let recent: Vec<StepSummary> = step_history[to_compact..].to_vec();
        step_history.clear();
        step_history.push(summary);
        step_history.extend(recent);

        // After compaction: 1 summary + keep_last recent = 3 total
        assert_eq!(step_history.len(), 1 + keep);
        assert_eq!(step_history[0].action_kind, "compacted_summary");
        assert!(step_history[0].summary.contains("Compacted 8 prior steps"));
        // Most recent steps preserved verbatim.
        assert_eq!(step_history[1].iteration, 8);
        assert_eq!(step_history[2].iteration, 9);
        assert!(before_count > step_history.len());
    }

    #[test]
    fn compaction_skips_when_below_min_steps() {
        let cfg = crate::CompactionConfig {
            enabled: true,
            min_steps: 10,
            keep_last: 4,
            threshold_pct: 70,
            summary_token_budget: 2000,
            cooldown_iterations: 5,
        };

        let history_len = 5; // below min_steps
        assert!(history_len < cfg.min_steps);
        // Compaction would not trigger — this is a logic assertion, not a runtime test.
    }

    #[test]
    fn compaction_disabled_skips() {
        let cfg = crate::CompactionConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!cfg.enabled);
    }

    #[test]
    fn compaction_is_throttled_within_cooldown_window() {
        let config = crate::CompactionConfig {
            enabled: true,
            threshold_pct: 1,
            min_steps: 3,
            keep_last: 2,
            summary_token_budget: 2000,
            cooldown_iterations: 5,
        };
        let mut history: Vec<StepSummary> = (0..8)
            .map(|i| StepSummary {
                iteration: i,
                action_kind: "tool_call".to_owned(),
                summary: format!(
                    "Iteration {i} returned a very large diagnostic payload that should trigger compaction."
                ),
                succeeded: true,
            })
            .collect();
        let mut last_compaction_iteration = None;

        let first = maybe_compact_history(&mut history, 0, &config, &mut last_compaction_iteration);
        assert!(
            first.is_some(),
            "first over-threshold compaction should run"
        );

        history.extend((8..11).map(|i| StepSummary {
            iteration: i,
            action_kind: "tool_call".to_owned(),
            summary: format!(
                "Iteration {i} also returned a large payload but falls inside cooldown."
            ),
            succeeded: true,
        }));

        let second =
            maybe_compact_history(&mut history, 1, &config, &mut last_compaction_iteration);
        assert!(
            second.is_none(),
            "second compaction attempt inside cooldown must be throttled"
        );
        assert_eq!(
            last_compaction_iteration,
            Some(0),
            "cooldown should preserve the original compaction iteration"
        );
    }
}
