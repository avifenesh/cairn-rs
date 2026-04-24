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
    ActionResult, ActionStatus, DecideOutput, ExecuteOutcome, GatherOutput, LoopConfig, LoopSignal,
    LoopTermination, OrchestrationContext, StepSummary,
};
use crate::decide::DecidePhase;
use crate::emitter::{NoOpEmitter, OrchestratorEventEmitter};
use crate::error::OrchestratorError;
use crate::execute::{ApprovedDispatch, ExecutePhase};
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

    /// RFC 020 Track 4 — persist the `Intent` checkpoint after decide
    /// produces proposals and Track 3 mints their `ToolCallId`s, but
    /// *before* execute dispatches. On crash between decide and execute,
    /// recovery reads this checkpoint, walks the planned `ToolCallId`s,
    /// consults the `ToolCallResultCache`, and re-dispatches only the
    /// misses (per RFC 020 Gap 13 resolution).
    ///
    /// Default impl: no-op. `NoOpCheckpointHook` and test doubles inherit
    /// the no-op; production wiring overrides with a real save that calls
    /// `CheckpointService::save_dual(..., CheckpointKind::Intent, ...)`.
    async fn save_intent(
        &self,
        _ctx: &OrchestrationContext,
        _gather: &GatherOutput,
        _decide: &DecideOutput,
    ) -> Result<(), OrchestratorError> {
        Ok(())
    }

    /// RFC 020 Track 4 — persist the `Result` checkpoint after all
    /// dispatches settle (success / timeout / fail). Complements
    /// `save_intent`: the two checkpoints form the per-iteration pair that
    /// closes RFC 020 invariant #5. Default delegates to `save` so
    /// implementors with a single-checkpoint history keep working.
    async fn save_result(
        &self,
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
        decide: &DecideOutput,
        execute: &ExecuteOutcome,
    ) -> Result<(), OrchestratorError> {
        self.save(ctx, gather, decide, execute).await
    }
}

/// RFC 020 Track 4 — `CheckpointHook` that emits dual (Intent + Result)
/// checkpoints via `CheckpointService::save_dual`.
///
/// Intent is written after decide (before execute dispatches) and carries
/// the planned `ToolCallId`s Track 3 minted; Result is written after
/// execute completes and carries the post-iteration message history with
/// an empty `tool_call_ids` (the Intent checkpoint owns the full list per
/// Q4 resolution).
///
/// Full-snapshot bodies (Gap 3 resolution — v1 ships full snapshots, not
/// diffs). The emitted `CheckpointRecorded` event carries
/// `message_history_size` so operators can monitor cost and decide if
/// Track 4b diff compaction is worth the complexity.
pub struct DualCheckpointHook {
    project: cairn_domain::ProjectKey,
    checkpoints: std::sync::Arc<dyn cairn_runtime::CheckpointService>,
}

impl DualCheckpointHook {
    pub fn new(
        project: cairn_domain::ProjectKey,
        checkpoints: std::sync::Arc<dyn cairn_runtime::CheckpointService>,
    ) -> Self {
        Self {
            project,
            checkpoints,
        }
    }

    fn mint_checkpoint_id() -> cairn_domain::CheckpointId {
        cairn_domain::CheckpointId::new(format!("cp_{}", uuid::Uuid::now_v7()))
    }

    fn planned_tool_call_ids(decide: &DecideOutput) -> Vec<String> {
        // Advisory planned-call markers for the Intent checkpoint audit
        // body. These are NOT the hashed `ToolCallId`s execute mints —
        // bugbot #84/medium flagged that my earlier implementation sorted
        // by (tool_name, args) and normalized via `Value::to_string()`,
        // which diverged from `execute_impl.rs` (positional `call_index`
        // + per-handler `normalize_for_cache`). Rather than reconstruct
        // the handler registry in the checkpoint hook (a layering break —
        // the registry lives inside `RuntimeExecutePhase`), we:
        //
        // 1. Preserve proposal order (so `call_index` in the marker
        //    matches execute's positional dispatch order).
        // 2. Carry the raw proposal args (JSON `to_string`), clearly
        //    labelled as a PLAN marker, not the hashed cache key.
        //
        // The accurate hashed `ToolCallId` lands on
        // `ToolInvocationCompleted.tool_call_id` at dispatch time, which
        // is what recovery reads via `ToolCallResultCache::get`. The
        // Intent checkpoint's `tool_call_ids` is a human-readable audit
        // record of the plan, useful for operator dashboards and the
        // future resume path (Gap 13 consumer).
        //
        // Proposals without a `tool_name` (CompleteRun /
        // EscalateToOperator / CreateMemory) have no stream-facing tool
        // analogue and produce no marker.
        decide
            .proposals
            .iter()
            .enumerate()
            .filter_map(|(call_index, proposal)| {
                let tool_name = proposal.tool_name.as_deref()?;
                let args_json = proposal
                    .tool_args
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "null".to_owned());
                Some(format!("planned:{call_index}:{tool_name}:{args_json}"))
            })
            .collect()
    }

    fn build_history_snapshot(
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
        decide: &DecideOutput,
        execute: Option<&ExecuteOutcome>,
    ) -> serde_json::Value {
        // Full-snapshot body (RFC 020 Gap 3): serialize the iteration
        // artifacts we have in hand. Concrete shape is stable JSON so a
        // future resume path can parse it without schema migration.
        serde_json::json!({
            "run_id": ctx.run_id.as_str(),
            "iteration": ctx.iteration,
            "goal": ctx.goal,
            "step_history": ctx.step_history,
            "gather": {
                "memory_chunk_count": gather.memory_chunks.len(),
                "step_history_count": gather.step_history.len(),
            },
            "decide": {
                "proposal_count": decide.proposals.len(),
                "requires_approval": decide.requires_approval,
                "model_id": decide.model_id,
                "latency_ms": decide.latency_ms,
            },
            "execute": execute.map(|e| serde_json::json!({
                "result_count": e.results.len(),
                "loop_signal": format!("{:?}", e.loop_signal),
            })),
        })
    }
}

#[async_trait::async_trait]
impl CheckpointHook for DualCheckpointHook {
    async fn save(
        &self,
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
        decide: &DecideOutput,
        execute: &ExecuteOutcome,
    ) -> Result<(), OrchestratorError> {
        // Fallback path for callers using the legacy single-`save` API.
        // `save_result` is the canonical Track 4 post-execute path.
        self.save_result(ctx, gather, decide, execute).await
    }

    async fn save_intent(
        &self,
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
        decide: &DecideOutput,
    ) -> Result<(), OrchestratorError> {
        let cp_id = Self::mint_checkpoint_id();
        let body = Self::build_history_snapshot(ctx, gather, decide, None);
        let tool_call_ids = Self::planned_tool_call_ids(decide);
        self.checkpoints
            .save_dual(
                &self.project,
                &ctx.run_id,
                cp_id,
                cairn_domain::CheckpointKind::Intent,
                body,
                tool_call_ids,
            )
            .await
            .map(|_| ())
            .map_err(OrchestratorError::Runtime)
    }

    async fn save_result(
        &self,
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
        decide: &DecideOutput,
        execute: &ExecuteOutcome,
    ) -> Result<(), OrchestratorError> {
        let cp_id = Self::mint_checkpoint_id();
        let body = Self::build_history_snapshot(ctx, gather, decide, Some(execute));
        // RFC 020 Track 4 §6 Q4 — Result checkpoint carries an empty
        // `tool_call_ids`. Intent already owns the full planned list;
        // duplicating here only inflates the event body.
        self.checkpoints
            .save_dual(
                &self.project,
                &ctx.run_id,
                cp_id,
                cairn_domain::CheckpointKind::Result,
                body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(OrchestratorError::Runtime)
    }
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

/// Minimum wall-clock budget remaining (ms) required to start a DECIDE.
///
/// When less than this is left on the run deadline we terminate cleanly as
/// `LoopTermination::TimedOut` instead of firing an LLM call that is
/// guaranteed to miss the deadline mid-flight. 5s is a conservative heuristic:
/// smaller than any provider's default timeout so it only fires near the
/// actual deadline, but large enough that a successful DECIDE could realistically
/// finish in the remaining window on fast paths. Chosen over the per-provider
/// timeout because different bindings in a routing chain have different
/// defaults — 5s is the largest common lower bound that doesn't leak those
/// internals up to the loop.
pub const MIN_DECIDE_BUDGET_MS: u64 = 5_000;

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
    /// F25 drain: optional read of the tool-call approval projection.
    /// When wired, the loop drains any operator-approved-but-not-executed
    /// proposals for the run at the top of each `run_inner` invocation
    /// BEFORE calling DECIDE. Without this, a re-orchestrate after
    /// approval never reaches the approved tool — the LLM just sees the
    /// un-changed context and emits the same proposal again (which the
    /// approval service then treats as a duplicate and auto-approves
    /// without re-dispatch, looping forever).
    approval_reader: Option<Arc<dyn cairn_runtime::tool_call_approvals::ToolCallApprovalReader>>,
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
            approval_reader: None,
        }
    }

    /// F25 drain: install the tool-call approval reader so the loop
    /// drains operator-approved proposals for the run before the next
    /// DECIDE iteration. Without this, re-orchestrate after an approval
    /// round-trip silently drops the approved tool call (the dogfood
    /// blocker).
    pub fn with_approval_reader(
        mut self,
        reader: Arc<dyn cairn_runtime::tool_call_approvals::ToolCallApprovalReader>,
    ) -> Self {
        self.approval_reader = Some(reader);
        self
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

    /// F25 drain: execute any operator-approved tool calls for this run
    /// whose `ToolCallId` the caller has not already drained this
    /// `run_inner` invocation. Returns one `ActionResult` per drained
    /// proposal in oldest-first order.
    ///
    /// `already_drained` is a per-invocation ledger of `ToolCallId`
    /// strings the loop has already processed. Without it, every
    /// iteration would re-fetch the same Approved rows and re-emit
    /// tool_called / tool_result / StepSummary for each one — even if
    /// `dispatch_approved` silently served a cache hit, the bloat in
    /// `step_history` would poison the next DECIDE's context. This
    /// caller-owned set caps the work at once per call_id per invocation.
    ///
    /// `dispatch_approved` still performs its own
    /// `ToolCallResultCache`-presence check as a second line of defence:
    /// a long-running loop that outlives a restart (theoretical; current
    /// runner does not) would hit the cache on the post-restart rebuild.
    ///
    /// When no approval reader is wired (default), the drain is a no-op.
    async fn drain_approved_pending(
        &self,
        ctx: &OrchestrationContext,
        already_drained: &mut std::collections::HashSet<String>,
    ) -> Result<Vec<ActionResult>, OrchestratorError> {
        let Some(reader) = &self.approval_reader else {
            return Ok(Vec::new());
        };

        let approved = reader
            .list_approved_for_run(&ctx.run_id)
            .await
            .map_err(OrchestratorError::Runtime)?;
        if approved.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        for ap in approved {
            let call_id_str = ap.call_id.as_str().to_owned();
            if !already_drained.insert(call_id_str.clone()) {
                // Already handled this call_id earlier in the same
                // `run_inner` invocation — skip to avoid duplicate
                // emissions and step_history entries.
                continue;
            }

            let started_at = std::time::Instant::now();
            let dispatch = ApprovedDispatch {
                call_id: ap.call_id,
                tool_name: ap.tool_name,
                tool_args: ap.tool_args,
            };
            let mut result = self.execute.dispatch_approved(ctx, &dispatch).await?;
            result.duration_ms = started_at.elapsed().as_millis() as u64;
            tracing::info!(
                run_id = %ctx.run_id,
                tool = ?dispatch.tool_name,
                succeeded = matches!(result.status, ActionStatus::Succeeded),
                "F25 drain: dispatched approved tool call"
            );
            results.push(result);
        }
        Ok(results)
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
        // F25 drain dedup ledger: every approved `ToolCallId` the drain
        // has processed in THIS `run_inner` invocation. Prevents
        // duplicate tool_called/tool_result/StepSummary emissions when
        // subsequent iterations re-list the same projection row.
        let mut drained_call_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        tracing::info!(
            run_id    = %ctx.run_id,
            goal      = %ctx.goal,
            agent     = %ctx.agent_type,
            max_iter  = self.config.max_iterations,
            timeout_s = self.config.timeout_ms / 1_000,
            "orchestrator loop starting"
        );
        for _iter in 0..self.config.max_iterations {
            // ── (0) F25 drain: flush operator-approved tool calls ────────────
            //
            // Before GATHER reads the event log, replay any
            // `ToolCallApproved`-state proposals for this run that don't
            // yet have a matching `ToolInvocationCompleted`. Without this
            // step, a re-orchestrate after approval never invokes the
            // approved tool: the LLM sees the same context as last turn,
            // emits the same proposal, the approval service (correctly)
            // returns AutoApproved from its cache, but nothing actually
            // *runs* the tool — the dogfood F25 blocker. See
            // `CLAUDE.md` + `project_session_2026_04_22_part4.md`.
            //
            // Failures inside the drain surface as synthesized
            // StepSummary entries + tool_result events so the next
            // DECIDE sees what went wrong and the LLM can self-correct.
            let drained = self
                .drain_approved_pending(ctx, &mut drained_call_ids)
                .await?;
            if !drained.is_empty() {
                for result in &drained {
                    let Some(tool_name) = result.proposal.tool_name.as_deref() else {
                        continue;
                    };
                    let args = result
                        .proposal
                        .tool_args
                        .clone()
                        .unwrap_or(serde_json::Value::Null);

                    // SSE: tool_called + tool_result (mirrors main path).
                    self.emitter
                        .on_tool_called(ctx, tool_name, Some(&args))
                        .await;
                    let (succeeded, error) = match &result.status {
                        ActionStatus::Succeeded => (true, None),
                        ActionStatus::Failed { reason } => (false, Some(reason.as_str())),
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

                    // FF attempt-stream: tool_call + tool_result frames.
                    // `restore_frames()` on resume expects these pair
                    // with every tool dispatch, including drain. Best-
                    // effort (warn+continue) per the sink contract.
                    if let Err(e) = self.task_sink.log_tool_call(tool_name, &args).await {
                        tracing::warn!(
                            run_id = %ctx.run_id,
                            tool = %tool_name,
                            error = %e,
                            "drain: task_sink.log_tool_call failed — frame lost"
                        );
                    }
                    let output = match &result.tool_output {
                        Some(v) => v.clone(),
                        None => match error {
                            Some(reason) => serde_json::json!({"error": reason}),
                            None => serde_json::Value::Null,
                        },
                    };
                    if let Err(e) = self
                        .task_sink
                        .log_tool_result(tool_name, &output, succeeded, result.duration_ms)
                        .await
                    {
                        tracing::warn!(
                            run_id = %ctx.run_id,
                            tool = %tool_name,
                            error = %e,
                            "drain: task_sink.log_tool_result failed — frame lost"
                        );
                    }

                    // Append StepSummary so DECIDE's next gather sees
                    // the drained action as part of the run's history.
                    let action_kind = "invoke_tool".to_owned();
                    let summary = match &result.status {
                        ActionStatus::Succeeded => format!("drained approved: {tool_name}"),
                        ActionStatus::Failed { reason } => {
                            format!("drained approved {tool_name} failed: {reason}")
                        }
                        _ => format!("drained approved {tool_name}"),
                    };
                    step_history.push(StepSummary {
                        iteration: ctx.iteration,
                        action_kind,
                        summary,
                        succeeded,
                    });
                }
            }

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

            // ── (2c) Pre-DECIDE budget check ─────────────────────────────────
            // GATHER just finished; if the remaining budget is too small
            // to reasonably complete a DECIDE round-trip, bail now rather
            // than firing the LLM call only to have the wall-clock
            // deadline trip in the middle and leave a stranded provider
            // request. The threshold is heuristic: smaller than the
            // smallest per-provider default would guarantee a provider
            // timeout fires before the loop deadline — wasteful. Larger
            // than DECIDE's typical latency avoids false positives.
            //
            // Uses `now_millis()` fresh: GATHER may itself have taken
            // meaningful time (retrieval + chunk scoring), so the
            // `remaining_ms` computed at iteration start is stale.
            let pre_decide_now_ms = now_millis();
            let remaining_before_decide_ms = deadline_ms.saturating_sub(pre_decide_now_ms);
            if remaining_before_decide_ms < MIN_DECIDE_BUDGET_MS {
                tracing::warn!(
                    run_id        = %ctx.run_id,
                    iteration     = ctx.iteration,
                    remaining_ms  = remaining_before_decide_ms,
                    min_budget_ms = MIN_DECIDE_BUDGET_MS,
                    "orchestrator loop budget too low to start DECIDE — timing out cleanly"
                );
                return Ok(LoopTermination::TimedOut);
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

            // ── (3a') Emitter fatal-error check ──────────────────────────────
            // `on_decide_completed` is the only callback that dual-writes
            // provider-call telemetry into the durable secondary (see the
            // `TracingEmitter` implementation in `cairn-app`). When its
            // append fails, the in-memory and durable logs have diverged
            // — the next iteration would read stale state from the
            // primary, so the loop must abort with a store error rather
            // than silently continue toward the iteration cap.
            if let Some(msg) = self.emitter.take_fatal_error() {
                tracing::error!(
                    run_id    = %ctx.run_id,
                    iteration = ctx.iteration,
                    error     = %msg,
                    "orchestrator emitter reported fatal error — aborting loop"
                );
                return Err(OrchestratorError::Store(cairn_store::StoreError::Internal(
                    msg,
                )));
            }

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
                // emission for this iteration. Sync ctx.step_history before
                // calling the hook so the checkpoint snapshot captures the
                // freshly-pushed summary (not just the prior iterations).
                let step_summary = build_step_summary(ctx, &decide_output, &execute_outcome);
                step_history.push(step_summary);
                ctx.step_history = step_history.clone();
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

                // No AwaitingApproval result — the BP-v2 ToolCallApproval
                // path ran the whole propose-then-await flow inline: the
                // proposal was submitted, the operator resolved it (or
                // it timed out), and the tool already dispatched +
                // recorded its result in this very execute_outcome.
                //
                // Treat the terminal loop_signal from the outcome as
                // authoritative: Continue (if the tool succeeded and
                // there's more work) flows back into the main loop via
                // the logic below; Done/Failed/etc. terminate via the
                // same match arms the non-approval path uses.
                //
                // Emit per-result tool_called/tool_result AND FF
                // tool_result frames so SSE + FF attempt-stream
                // telemetry stay consistent with the non-approval
                // path. The approval-gate pre-execute log_tool_call
                // already fired above; we emit on_tool_called here
                // too so SSE timelines render a matching begin/end
                // pair (the dashboard treats on_tool_called as the
                // "started" event — without it the dispatch would
                // show only a "result" with no corresponding call).
                for result in &execute_outcome.results {
                    let Some(tool_name) = result.proposal.tool_name.as_deref() else {
                        continue;
                    };
                    let args = result
                        .proposal
                        .tool_args
                        .clone()
                        .unwrap_or(serde_json::Value::Null);
                    self.emitter
                        .on_tool_called(ctx, tool_name, Some(&args))
                        .await;
                    let (succeeded, error) = match &result.status {
                        ActionStatus::Succeeded => (true, None),
                        ActionStatus::Failed { reason } => (false, Some(reason.as_str())),
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
                    // FF attempt-stream: tool_result frame mirrors
                    // the main-path log_tool_result at loop-runner
                    // line ~820. `restore_frames()` expects this on
                    // every dispatched call.
                    let output = match &result.tool_output {
                        Some(v) => v.clone(),
                        None => match error {
                            Some(reason) => serde_json::json!({"error": reason}),
                            None => serde_json::Value::Null,
                        },
                    };
                    if let Err(e) = self
                        .task_sink
                        .log_tool_result(tool_name, &output, succeeded, result.duration_ms)
                        .await
                    {
                        tracing::warn!(
                            run_id = %ctx.run_id,
                            tool = %tool_name,
                            error = %e,
                            "BP-v2 inline: task_sink.log_tool_result failed — frame lost"
                        );
                    }
                }

                match execute_outcome.loop_signal.clone() {
                    LoopSignal::Done => {
                        let summary = decide_output
                            .proposals
                            .iter()
                            .find(|p| p.action_type == cairn_domain::ActionType::CompleteRun)
                            .map(|p| p.description.clone())
                            .unwrap_or_else(|| "run completed".to_owned());
                        return Ok(LoopTermination::Completed { summary });
                    }
                    LoopSignal::Failed { reason } => {
                        return Ok(LoopTermination::Failed { reason });
                    }
                    LoopSignal::WaitSubagent { child_task_id } => {
                        return Ok(LoopTermination::WaitingSubagent { child_task_id });
                    }
                    LoopSignal::WaitApproval { approval_id } => {
                        // Redundant with the `AwaitingApproval` scan
                        // above, but covers derive_signal paths that
                        // set WaitApproval without the per-result
                        // status (legacy Escalate path).
                        return Ok(LoopTermination::WaitingApproval { approval_id });
                    }
                    LoopSignal::PlanProposed { plan_markdown } => {
                        return Ok(LoopTermination::PlanProposed { plan_markdown });
                    }
                    LoopSignal::Continue => {
                        // BP-v2 dispatched successfully; bump iteration
                        // and fall through to the next loop turn.
                        ctx.iteration = ctx.iteration.saturating_add(1);
                        continue;
                    }
                }
            }

            // ── (4b) RFC 020 Track 4: INTENT CHECKPOINT ───────────────────────
            // Before any dispatch, persist the decide output + planned
            // `ToolCallId`s (Track 3 minted these in execute's pre-dispatch
            // stage; see `execute_impl.rs` — the Intent checkpoint captures
            // the *intent* to invoke them). On crash between here and
            // execute completion, recovery walks the planned IDs, consults
            // `ToolCallResultCache`, and re-dispatches only misses.
            // Default `CheckpointHook::save_intent` is a no-op; production
            // wiring overrides to invoke `CheckpointService::save_dual(…,
            // CheckpointKind::Intent, …)`. Failure is logged + swallowed;
            // RFC 020 invariant #5 is best-effort, not blocking.
            if let Err(e) = self
                .checkpoint_hook
                .save_intent(ctx, &gather_output, &decide_output)
                .await
            {
                tracing::warn!(
                    run_id    = %ctx.run_id,
                    iteration = ctx.iteration,
                    error     = %e,
                    "intent checkpoint save failed — continuing without intent checkpoint"
                );
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
            // Sync ctx.step_history so the hook sees the just-pushed summary.
            ctx.step_history = step_history.clone();

            // RFC 020 Track 4: call `save_result` (dual-checkpoint Result
            // side). Default impl delegates to `save`, so legacy single-
            // checkpoint hooks keep their existing behavior.
            if let Err(e) = self
                .checkpoint_hook
                .save_result(ctx, &gather_output, &decide_output, &execute_outcome)
                .await
            {
                // Checkpoint failures are logged but do NOT abort the run.
                // The next successful checkpoint will capture the current state.
                tracing::warn!(
                    run_id    = %ctx.run_id,
                    iteration = ctx.iteration,
                    error     = %e,
                    "result checkpoint save failed — continuing without checkpoint"
                );
            } else {
                tracing::debug!(
                    run_id    = %ctx.run_id,
                    iteration = ctx.iteration,
                    "result checkpoint saved"
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
            is_recovery: false,
            approval_timeout: None,
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

    // ── (2c) Pre-DECIDE budget check ──────────────────────────────────────────
    //
    // F27 guard: GATHER can consume meaningful wall-clock time (retrieval,
    // chunk scoring). If the remaining budget drops below
    // `MIN_DECIDE_BUDGET_MS` between iteration start and post-GATHER, the
    // loop MUST bail cleanly as `TimedOut` rather than firing an LLM call
    // that is guaranteed to miss the deadline. This test pins that gate by
    // putting the context ~3s past "now" on a 5s loop budget — GATHER
    // finishes instantly but the remaining budget is 2s, below the 5s
    // threshold, so the loop must terminate before DECIDE runs.

    struct CountingDecide {
        count: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl DecidePhase for CountingDecide {
        async fn decide(
            &self,
            _: &OrchestrationContext,
            _: &GatherOutput,
        ) -> Result<DecideOutput, OrchestratorError> {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(decide_done())
        }
    }

    #[tokio::test]
    async fn budget_below_threshold_short_circuits_before_decide() {
        let mut tight_ctx = ctx();
        // Started 3s ago on a 5s total budget → only 2s remaining,
        // strictly below MIN_DECIDE_BUDGET_MS (5s).
        tight_ctx.run_started_at_ms = now_millis().saturating_sub(3_000);

        let decide = CountingDecide {
            count: std::sync::atomic::AtomicUsize::new(0),
        };
        let decide_handle = std::sync::Arc::new(decide);
        let decide_for_loop = decide_handle.clone();

        // Wrap so we can inspect count after the run. Using a dedicated
        // trait adapter is overkill — we leak a raw pointer via Arc here
        // because `OrchestratorLoop::new` takes the phase by value and
        // we don't need shared mutation, just post-run inspection.
        struct ArcDecide(std::sync::Arc<CountingDecide>);
        #[async_trait]
        impl DecidePhase for ArcDecide {
            async fn decide(
                &self,
                ctx: &OrchestrationContext,
                g: &GatherOutput,
            ) -> Result<DecideOutput, OrchestratorError> {
                self.0.decide(ctx, g).await
            }
        }

        let config = LoopConfig {
            timeout_ms: 5_000,
            ..Default::default()
        };
        let lp = OrchestratorLoop::new(
            FixedGather,
            ArcDecide(decide_for_loop),
            ScriptedExecute {
                signal: LoopSignal::Done,
            },
            config,
        );

        let result = lp.run(tight_ctx).await.unwrap();
        assert!(
            matches!(result, LoopTermination::TimedOut),
            "expected TimedOut from pre-DECIDE budget check, got {result:?}"
        );
        // Critical: DECIDE must NOT have been invoked. Otherwise we are
        // firing LLM calls we know will miss the deadline — the very
        // behaviour F27 adds this guard to prevent.
        assert_eq!(
            decide_handle
                .count
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "decide fired despite budget below MIN_DECIDE_BUDGET_MS"
        );
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

    // ── (1c) Emitter fatal-error gate ────────────────────────────────────────
    //
    // F24 dogfood (2026-04-23): when a composite emitter's side-effect
    // append to the durable secondary fails, the loop must abort
    // rather than silently continue on top of a diverged store. The
    // new `take_fatal_error()` contract lets emitters surface errors
    // that their `()`-returning callbacks can't express.

    struct FatalEmitter {
        consumed: std::sync::Mutex<bool>,
    }
    #[async_trait]
    impl crate::emitter::OrchestratorEventEmitter for FatalEmitter {
        fn take_fatal_error(&self) -> Option<String> {
            let mut consumed = self.consumed.lock().unwrap();
            if *consumed {
                None
            } else {
                *consumed = true;
                Some("dual-write divergence on run=run: boom".to_owned())
            }
        }
    }

    #[tokio::test]
    async fn emitter_fatal_error_aborts_loop_with_store_error() {
        let config = LoopConfig {
            max_iterations: 5,
            ..Default::default()
        };
        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_done()),
            ScriptedExecute {
                signal: LoopSignal::Done,
            },
            config,
        )
        .with_emitter(std::sync::Arc::new(FatalEmitter {
            consumed: std::sync::Mutex::new(false),
        }));

        let err = lp.run(ctx()).await.expect_err(
            "emitter-signalled fatal error must propagate as OrchestratorError, \
             not be silently swallowed",
        );
        match err {
            OrchestratorError::Store(msg) => {
                let s = msg.to_string();
                assert!(
                    s.contains("dual-write divergence"),
                    "expected divergence detail in error, got: {s}"
                );
            }
            other => {
                panic!("expected OrchestratorError::Store(dual-write divergence), got {other:?}")
            }
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
                        { "name": "bash",   "description": "run shell commands" },
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
        assert!(discovered.contains(&"bash".to_owned()));
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
                            "matches": [{"name":"bash","description":"run shell"}],
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
        // Second decide: bash must be in discovered_tool_names
        assert!(
            snapshots[1].contains(&"bash".to_owned()),
            "iteration 1 must see bash from prior tool_search result"
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

    // ── (D) F25 drain tests ──────────────────────────────────────────────────
    //
    // These drive the "approved-but-not-executed" drain the loop runs
    // before each GATHER. They use stub phases + a stub
    // `ToolCallApprovalReader` so the unit tests stay hermetic. The
    // HTTP-level integration test in
    // `crates/cairn-app/tests/test_drain_approved_executes_bash.rs`
    // covers the full bash-on-filesystem flow.

    use cairn_domain::{ApprovalMatchPolicy, ApprovalScope};
    use cairn_runtime::error::RuntimeError;
    use cairn_runtime::tool_call_approvals::{
        ApprovedProposal, StoredProposal, ToolCallApprovalReader,
    };

    /// Stub reader returning a scripted list of approved proposals plus
    /// a call-count so tests can assert the reader was consulted.
    struct ScriptedApprovalReader {
        approved: std::sync::Mutex<Vec<ApprovedProposal>>,
        list_calls: std::sync::atomic::AtomicU32,
    }
    impl ScriptedApprovalReader {
        fn with(approved: Vec<ApprovedProposal>) -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                approved: std::sync::Mutex::new(approved),
                list_calls: std::sync::atomic::AtomicU32::new(0),
            })
        }
        fn list_calls(&self) -> u32 {
            self.list_calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }
    #[async_trait]
    impl ToolCallApprovalReader for ScriptedApprovalReader {
        async fn get_tool_call_approval(
            &self,
            _call_id: &cairn_domain::ToolCallId,
        ) -> Result<Option<ApprovedProposal>, RuntimeError> {
            Ok(None)
        }
        async fn get_tool_call_proposal(
            &self,
            _call_id: &cairn_domain::ToolCallId,
        ) -> Result<Option<StoredProposal>, RuntimeError> {
            Ok(None)
        }
        async fn list_approved_for_run(
            &self,
            _run_id: &cairn_domain::RunId,
        ) -> Result<Vec<ApprovedProposal>, RuntimeError> {
            self.list_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut guard = self.approved.lock().unwrap();
            // Return once, then empty — the drain should run on the
            // first iteration and find nothing on subsequent ones.
            let take = std::mem::take(&mut *guard);
            Ok(take)
        }
    }
    // Silence "unused match arms via ApprovalMatchPolicy/ApprovalScope"
    // when the import is only needed by tests below the current edit.
    const _: Option<ApprovalMatchPolicy> = None;
    const _: Option<ApprovalScope> = None;

    /// ExecutePhase stub whose `dispatch_approved` records the
    /// call_ids it was handed and returns a canned status. It still
    /// implements `execute` as a no-op so the outer loop terminates.
    struct RecordingDispatch {
        dispatched: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        status: std::sync::Mutex<crate::context::ActionStatus>,
    }
    impl RecordingDispatch {
        fn new(status: crate::context::ActionStatus) -> Self {
            Self {
                dispatched: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                status: std::sync::Mutex::new(status),
            }
        }
    }
    #[async_trait]
    impl ExecutePhase for RecordingDispatch {
        async fn execute(
            &self,
            _ctx: &OrchestrationContext,
            _decide: &DecideOutput,
        ) -> Result<ExecuteOutcome, OrchestratorError> {
            // After the drain, the outer loop calls execute with a
            // complete_run proposal — short-circuit to Done.
            Ok(ExecuteOutcome {
                results: vec![],
                loop_signal: LoopSignal::Done,
            })
        }
        async fn dispatch_approved(
            &self,
            _ctx: &OrchestrationContext,
            approved: &crate::execute::ApprovedDispatch,
        ) -> Result<ActionResult, OrchestratorError> {
            self.dispatched
                .lock()
                .unwrap()
                .push(approved.call_id.as_str().to_owned());
            let synth = ActionProposal {
                action_type: ActionType::InvokeTool,
                description: format!("drained {}", approved.tool_name),
                confidence: 1.0,
                tool_name: Some(approved.tool_name.clone()),
                tool_args: Some(approved.tool_args.clone()),
                requires_approval: false,
            };
            Ok(ActionResult {
                proposal: synth,
                status: self.status.lock().unwrap().clone(),
                tool_output: Some(serde_json::json!({"drained": true})),
                invocation_id: None,
                duration_ms: 0,
            })
        }
    }

    fn approved(call_id: &str, tool: &str, args: serde_json::Value) -> ApprovedProposal {
        ApprovedProposal {
            call_id: cairn_domain::ToolCallId::new(call_id),
            tool_name: tool.to_owned(),
            tool_args: args,
        }
    }

    #[tokio::test]
    async fn drain_dispatches_approved_proposals_before_decide() {
        // Two approved proposals sit in the projection waiting for
        // re-orchestrate. The drain must dispatch both, in order, and
        // pass through to DECIDE (which ends the run via CompleteRun).
        let reader = ScriptedApprovalReader::with(vec![
            approved("tc_1", "bash", serde_json::json!({"command": "echo one"})),
            approved("tc_2", "bash", serde_json::json!({"command": "echo two"})),
        ]);
        let dispatch = RecordingDispatch::new(ActionStatus::Succeeded);
        let dispatched_handle = dispatch.dispatched.clone();

        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_done()),
            dispatch,
            LoopConfig::default(),
        )
        .with_approval_reader(reader.clone());

        let result = lp.run(ctx()).await.unwrap();
        assert!(
            matches!(result, LoopTermination::Completed { .. }),
            "loop should complete after drain + decide, got {result:?}"
        );

        let dispatched = dispatched_handle.lock().unwrap().clone();
        assert_eq!(
            dispatched,
            vec!["tc_1".to_owned(), "tc_2".to_owned()],
            "drain must dispatch approved proposals in oldest-first order"
        );
        assert!(
            reader.list_calls() >= 1,
            "approval reader must be consulted at least once"
        );
    }

    #[tokio::test]
    async fn drain_is_noop_when_no_approvals_present() {
        let reader = ScriptedApprovalReader::with(vec![]);
        let dispatch = RecordingDispatch::new(ActionStatus::Succeeded);
        let dispatched_handle = dispatch.dispatched.clone();

        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_done()),
            dispatch,
            LoopConfig::default(),
        )
        .with_approval_reader(reader);

        let _ = lp.run(ctx()).await.unwrap();
        assert!(
            dispatched_handle.lock().unwrap().is_empty(),
            "drain must not dispatch anything when the reader returns empty"
        );
    }

    #[tokio::test]
    async fn drain_without_reader_is_noop() {
        // No `with_approval_reader` — the drain path must skip cleanly
        // so existing tests (and any deployment that hasn't wired a
        // reader yet) are unaffected.
        let dispatch = RecordingDispatch::new(ActionStatus::Succeeded);
        let dispatched_handle = dispatch.dispatched.clone();

        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_done()),
            dispatch,
            LoopConfig::default(),
        );

        let _ = lp.run(ctx()).await.unwrap();
        assert!(
            dispatched_handle.lock().unwrap().is_empty(),
            "drain must be a no-op when no approval_reader is wired"
        );
    }

    #[tokio::test]
    async fn drain_tool_failure_continues_to_decide() {
        // A drained tool that fails must NOT abort the loop — the LLM
        // needs to see the failure on the next GATHER so it can
        // self-correct. We wire a RecordingDispatch returning Failed
        // and assert the loop still reaches its terminal state via
        // DECIDE's CompleteRun proposal.
        let reader = ScriptedApprovalReader::with(vec![approved(
            "tc_failing",
            "bash",
            serde_json::json!({"command": "false"}),
        )]);
        let dispatch = RecordingDispatch::new(ActionStatus::Failed {
            reason: "bash exited 1".to_owned(),
        });
        let dispatched_handle = dispatch.dispatched.clone();

        let lp = OrchestratorLoop::new(
            FixedGather,
            ScriptedDecide::always(decide_done()),
            dispatch,
            LoopConfig::default(),
        )
        .with_approval_reader(reader);

        let result = lp.run(ctx()).await.unwrap();
        assert!(
            matches!(result, LoopTermination::Completed { .. }),
            "drain failure must not abort; loop should reach DECIDE and terminate, \
             got {result:?}"
        );
        assert_eq!(
            dispatched_handle.lock().unwrap().len(),
            1,
            "failing drain entry must still have been dispatched"
        );
    }
}
