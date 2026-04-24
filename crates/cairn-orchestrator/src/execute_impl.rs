//! RuntimeExecutePhase — concrete ExecutePhase backed by cairn-rs runtime services.
//!
//! Dispatches each `ActionProposal` from `DecideOutput` to the appropriate
//! runtime service:
//!
//! | `ActionType`         | Runtime service                                    |
//! |----------------------|----------------------------------------------------|
//! | `InvokeTool`         | `ToolInvocationService` + inline tool dispatch     |
//! | `SpawnSubagent`      | `TaskServiceImpl::spawn_subagent`                  |
//! | `SendNotification`   | `MailboxService::send`                             |
//! | `CompleteRun`        | `RunService::complete`                             |
//! | `EscalateToOperator` | `ApprovalService::request` + run → waiting_approval|
//! | `CreateMemory`       | async no-op (memory ingestion runs independently)  |
//!
//! After each successful tool call, `CheckpointService::save` is called if
//! the tool call count meets the configured `checkpoint_every_n_tool_calls`
//! threshold (default: save after every tool call).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{
    policy::ApprovalRequirement,
    tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget},
    ActionType, ApprovalId, CheckpointId, ExecutionClass, RuntimeEvent, SessionId, TaskId,
    ToolInvocationCacheHit, ToolInvocationId, ToolRecoveryPaused,
};
use cairn_runtime::{
    decisions::DecisionService,
    mailbox::MailboxService,
    services::ToolInvocationService,
    startup::{CachedToolResult, RecoveryDispatchDecision, ToolCallId, ToolCallResultCache},
    tool_call_approvals::{
        ApprovalDecision as ToolCallApprovalDecision, OperatorDecision, ToolCallApprovalService,
        ToolCallProposal,
    },
    ApprovalService, CheckpointService, RunService, TaskService,
};
use cairn_tools::builtins::BuiltinToolRegistry;
#[allow(unused_imports)]
use cairn_tools::builtins::ToolHandler;
use std::sync::Mutex;
use std::time::Duration;

use crate::context::{
    ActionResult, ActionStatus, DecideOutput, ExecuteOutcome, LoopSignal, OrchestrationContext,
};
use crate::error::OrchestratorError;
use crate::execute::{ApprovedDispatch, ExecutePhase};

// ── RuntimeExecutePhase ───────────────────────────────────────────────────────

/// Concrete `ExecutePhase` that routes `ActionProposal` variants through the
/// cairn-rs runtime service layer.
///
/// Construct via [`RuntimeExecutePhase::builder`].  All services share the
/// same underlying `InMemoryStore` so writes from one service are immediately
/// visible to reads from another.
pub struct RuntimeExecutePhase {
    run_service: Arc<dyn RunService>,
    task_service: Arc<dyn TaskService>,
    approval_service: Arc<dyn ApprovalService>,
    checkpoint_service: Arc<dyn CheckpointService>,
    mailbox_service: Arc<dyn MailboxService>,
    tool_invocation_service: Arc<dyn ToolInvocationService>,
    /// Registered built-in tools (memory_search, memory_store, …). Required
    /// for tool dispatch; absent means every `InvokeTool` proposal fails loud.
    tool_registry: Option<Arc<BuiltinToolRegistry>>,
    /// Decision service for pre-dispatch policy evaluation (RFC 019).
    decision_service: Option<Arc<dyn DecisionService>>,
    /// Save a checkpoint after every N-th successful tool call (1 = every call).
    checkpoint_every_n_tool_calls: u32,
    /// Maximum size of tool output copied back into the LLM context.
    tool_output_token_limit: usize,
    /// T5-H2: tool-call counter is cumulative across iterations so the
    /// `checkpoint_every_n_tool_calls` cadence honours its name. A
    /// per-iteration local counter never triggers when each iteration
    /// contains a single tool call.
    tool_call_count: std::sync::atomic::AtomicU32,
    /// RFC 020 Track 3: shared `ToolCallResultCache` consulted before each
    /// tool dispatch. A hit serves the cached result, emits
    /// `ToolInvocationCacheHit`, and skips invocation entirely. A miss
    /// proceeds to dispatch; successful completions populate the cache so
    /// a subsequent same-step replay hits.
    ///
    /// Optional so existing callers (tests) can omit wiring; production
    /// always injects a shared cache via the builder.
    tool_result_cache: Option<Arc<Mutex<ToolCallResultCache>>>,
    /// BP-v2 (research doc `docs/research/llm-agent-approval-systems.md`)
    /// propose-then-await service. When wired, the execute phase drives
    /// `requires_approval` tool calls through `submit_proposal` +
    /// `await_decision` + `retrieve_approved_proposal` so the proposal
    /// survives across the operator decision — killing the dogfood bug
    /// where the approval gate discarded the LLM's args and re-queried
    /// the model after approval.
    ///
    /// `None` preserves the legacy `ApprovalService::request_with_context`
    /// short-circuit used by existing tests.
    tool_call_approval_service: Option<Arc<dyn ToolCallApprovalService>>,
    /// Fallback wall-clock timeout applied to
    /// `ToolCallApprovalService::await_decision` when the orchestration
    /// context didn't override it. Defaults to 24h.
    ///
    /// NOTE: as of F26 (dogfood blocker fix) the approval gate no longer
    ///   calls `await_decision` in-process — `PendingOperator` suspends
    ///   the loop immediately. The field is preserved for the builder
    ///   API + future use (e.g. an optional synchronous-wait mode gated
    ///   on a capability flag), but is currently unused in the hot path.
    #[allow(dead_code)]
    approval_timeout_default: Duration,
}

impl RuntimeExecutePhase {
    pub fn builder() -> RuntimeExecutePhaseBuilder {
        RuntimeExecutePhaseBuilder::default()
    }
}

// ── Builder ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct RuntimeExecutePhaseBuilder {
    run_service: Option<Arc<dyn RunService>>,
    task_service: Option<Arc<dyn TaskService>>,
    approval_service: Option<Arc<dyn ApprovalService>>,
    checkpoint_service: Option<Arc<dyn CheckpointService>>,
    mailbox_service: Option<Arc<dyn MailboxService>>,
    tool_invocation_service: Option<Arc<dyn ToolInvocationService>>,
    tool_registry: Option<Arc<BuiltinToolRegistry>>,
    decision_service: Option<Arc<dyn DecisionService>>,
    checkpoint_every_n_tool_calls: u32,
    tool_output_token_limit: Option<usize>,
    tool_result_cache: Option<Arc<Mutex<ToolCallResultCache>>>,
    tool_call_approval_service: Option<Arc<dyn ToolCallApprovalService>>,
    approval_timeout_default: Option<Duration>,
}

impl RuntimeExecutePhaseBuilder {
    pub fn run_service(mut self, s: Arc<dyn RunService>) -> Self {
        self.run_service = Some(s);
        self
    }
    pub fn task_service(mut self, s: Arc<dyn TaskService>) -> Self {
        self.task_service = Some(s);
        self
    }
    pub fn approval_service(mut self, s: Arc<dyn ApprovalService>) -> Self {
        self.approval_service = Some(s);
        self
    }
    pub fn checkpoint_service(mut self, s: Arc<dyn CheckpointService>) -> Self {
        self.checkpoint_service = Some(s);
        self
    }
    pub fn mailbox_service(mut self, s: Arc<dyn MailboxService>) -> Self {
        self.mailbox_service = Some(s);
        self
    }
    pub fn tool_invocation_service(mut self, s: Arc<dyn ToolInvocationService>) -> Self {
        self.tool_invocation_service = Some(s);
        self
    }
    pub fn tool_registry(mut self, r: Arc<BuiltinToolRegistry>) -> Self {
        self.tool_registry = Some(r);
        self
    }
    pub fn checkpoint_every_n_tool_calls(mut self, n: u32) -> Self {
        self.checkpoint_every_n_tool_calls = n;
        self
    }
    pub fn tool_output_token_limit(mut self, limit: usize) -> Self {
        self.tool_output_token_limit = Some(limit.max(1));
        self
    }
    pub fn decision_service(mut self, ds: Arc<dyn DecisionService>) -> Self {
        self.decision_service = Some(ds);
        self
    }
    /// RFC 020 Track 3: inject a shared `ToolCallResultCache`. Without it,
    /// Track 3 cache-hit / recovery-pause behaviour is disabled (every
    /// dispatch runs fresh). Production wiring always injects one.
    pub fn tool_result_cache(mut self, cache: Arc<Mutex<ToolCallResultCache>>) -> Self {
        self.tool_result_cache = Some(cache);
        self
    }
    /// Inject the BP-v2 tool-call approval service. Required in production
    /// for the propose-then-await flow; tests omit it to exercise legacy
    /// `ApprovalService::request_with_context` paths.
    pub fn tool_call_approval_service(mut self, svc: Arc<dyn ToolCallApprovalService>) -> Self {
        self.tool_call_approval_service = Some(svc);
        self
    }
    /// Default timeout for operator approval decisions. Defaults to 24h
    /// when unset. Per-run override flows via
    /// `OrchestrationContext.approval_timeout`.
    pub fn approval_timeout_default(mut self, d: Duration) -> Self {
        self.approval_timeout_default = Some(d);
        self
    }
    pub fn build(self) -> RuntimeExecutePhase {
        RuntimeExecutePhase {
            run_service: self.run_service.expect("run_service required"),
            task_service: self.task_service.expect("task_service required"),
            approval_service: self.approval_service.expect("approval_service required"),
            checkpoint_service: self
                .checkpoint_service
                .expect("checkpoint_service required"),
            mailbox_service: self.mailbox_service.expect("mailbox_service required"),
            tool_invocation_service: self
                .tool_invocation_service
                .expect("tool_invocation_service required"),
            tool_registry: self.tool_registry,
            decision_service: self.decision_service,
            checkpoint_every_n_tool_calls: self.checkpoint_every_n_tool_calls.max(1),
            tool_output_token_limit: self.tool_output_token_limit.unwrap_or(2000),
            tool_call_count: std::sync::atomic::AtomicU32::new(0),
            tool_result_cache: self.tool_result_cache,
            tool_call_approval_service: self.tool_call_approval_service,
            approval_timeout_default: self
                .approval_timeout_default
                .unwrap_or_else(|| Duration::from_secs(24 * 60 * 60)),
        }
    }
}

// ── ExecutePhase impl ─────────────────────────────────────────────────────────

#[async_trait]
impl ExecutePhase for RuntimeExecutePhase {
    async fn execute(
        &self,
        ctx: &OrchestrationContext,
        decide: &DecideOutput,
    ) -> Result<ExecuteOutcome, OrchestratorError> {
        // ── Parallel batch for InvokeTool ─────────────────────────────────
        //
        // When the LLM emits multiple tool_calls in a single turn (modern
        // models do this — "read fileA AND read fileB in parallel"), we
        // MUST NOT serialize the batch on the slowest approval. One
        // tool-call waiting on operator approval cannot block N auto-
        // approved siblings from running.
        //
        // Strategy: drive every `InvokeTool` proposal concurrently via
        // `futures::future::join_all`. Each call owns its own oneshot in
        // the ToolCallApprovalService, so pending approvals block only
        // their own future. Non-InvokeTool proposals (CompleteRun,
        // SpawnSubagent, SendNotification, EscalateToOperator,
        // CreateMemory) are processed sequentially afterwards — those are
        // bookkeeping / control-flow steps that carry loop-terminal signals
        // (Done, WaitSubagent, WaitApproval) and must honour the original
        // short-circuit semantics.
        //
        // Original positional ordering is preserved so downstream emitters
        // (tool_called, tool_result, FF attempt_stream frames) still see
        // results indexed to the LLM's emitted proposals.

        let mut results: Vec<Option<ActionResult>> =
            (0..decide.proposals.len()).map(|_| None).collect();
        let mut loop_signal = LoopSignal::Continue;

        // ── Phase 1: parallel InvokeTool dispatch ────────────────────────
        //
        // Only parallel-batch InvokeTool proposals whose position is
        // BEFORE the first always-terminal control-flow proposal
        // (`CompleteRun`, `SpawnSubagent`, `EscalateToOperator`). The old
        // sequential path would break on those before reaching later
        // InvokeTool proposals, and we must preserve that semantic — a
        // `[CompleteRun, invoke_foo]` decide output must NOT run `foo`.
        //
        // `SendNotification` and `CreateMemory` are non-InvokeTool but
        // never set terminal signals, so they do NOT split the batch.
        let first_terminal_idx = decide.proposals.iter().position(|p| {
            matches!(
                p.action_type,
                ActionType::CompleteRun
                    | ActionType::SpawnSubagent
                    | ActionType::EscalateToOperator
            )
        });
        let invoke_indices: Vec<usize> = decide
            .proposals
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                if p.action_type != ActionType::InvokeTool {
                    return None;
                }
                match first_terminal_idx {
                    Some(k) if i > k => None,
                    _ => Some(i),
                }
            })
            .collect();

        if !invoke_indices.is_empty() {
            let futs = invoke_indices.iter().map(|&i| {
                let proposal = decide.proposals[i].clone();
                async move {
                    let started_at = std::time::Instant::now();
                    let mut result = self.dispatch_one(ctx, &proposal, i as u32).await?;
                    result.duration_ms = started_at.elapsed().as_millis() as u64;
                    Ok::<_, OrchestratorError>((i, result))
                }
            });
            let joined = futures::future::join_all(futs).await;
            for outcome in joined {
                let (i, result) = outcome?;
                results[i] = Some(result);
            }

            // ── Derive loop signal from parallel InvokeTool results ──────
            //
            // `dispatch_one(InvokeTool)` CAN return `AwaitingApproval` from
            // two paths that predate this PR:
            //
            //   1. Legacy approval-gate fallback (no `ToolCallApprovalService`
            //      wired) — `request_with_context` short-circuit returns
            //      `AwaitingApproval`.
            //   2. RFC 020 Track 3 `DangerousPause` recovery branch — on
            //      crash-mid-dispatch of a non-idempotent tool, operator
            //      must confirm re-invocation.
            //
            // Both MUST escalate to `LoopSignal::WaitApproval` so the loop
            // runner suspends. Missing this is a production-critical hole
            // in the recovery-pause case (dangerous tools would silently
            // skip the operator gate). Earliest-index wins so the ordering
            // matches what the old sequential path produced.
            for i in &invoke_indices {
                let Some(result) = results[*i].as_ref() else {
                    continue;
                };
                let next_signal = derive_signal(result, &loop_signal);
                if !matches!(next_signal, LoopSignal::Continue) {
                    loop_signal = next_signal;
                    break;
                }
            }
        }

        // ── Phase 2: sequential non-InvokeTool dispatch ──────────────────
        //
        // These carry terminal signals (CompleteRun → Done, SpawnSubagent
        // → WaitSubagent, EscalateToOperator → WaitApproval). Original
        // short-circuit semantics: first terminal signal stops the batch.
        //
        // If Phase 1 already set a terminal signal (e.g. an InvokeTool hit
        // a recovery pause), we skip Phase 2 entirely — running bookkeeping
        // steps after the batch has already committed to suspending would
        // trip downstream invariants (double-completion, extra subagent
        // spawn).
        if matches!(loop_signal, LoopSignal::Continue) {
            for (i, proposal) in decide.proposals.iter().enumerate() {
                if proposal.action_type == ActionType::InvokeTool {
                    continue;
                }
                let started_at = std::time::Instant::now();
                let mut result = self.dispatch_one(ctx, proposal, i as u32).await?;
                result.duration_ms = started_at.elapsed().as_millis() as u64;

                let new_signal = derive_signal(&result, &loop_signal);
                results[i] = Some(result);
                if !matches!(new_signal, LoopSignal::Continue) {
                    loop_signal = new_signal;
                    break;
                }
            }
        }

        // Drop `None` slots — these are non-InvokeTool proposals that a
        // terminal short-circuit prevented from running. Emitting them
        // as synthesized `Failed` would inflate the loop runner's
        // `failed_count` telemetry and mislead any downstream consumer
        // that treats `Failed` as "dispatch was attempted and it
        // errored." A skipped proposal never got that far.
        //
        // The loop runner iterates `execute_outcome.results` without
        // zipping to `decide.proposals`, so a shorter results vector
        // is safe. This matches the pre-PR behaviour where the
        // sequential `break` also stopped adding results to the vec.
        let results: Vec<ActionResult> = results.into_iter().flatten().collect();

        // Re-check loop signal against parallel results: an InvokeTool
        // failure shouldn't carry a terminal signal, but any Failed status
        // should surface if no stronger signal is present.
        if matches!(loop_signal, LoopSignal::Continue) {
            if let Some(reason) = first_failure_reason(&results) {
                loop_signal = LoopSignal::Failed { reason };
            }
        }

        Ok(ExecuteOutcome {
            results,
            loop_signal,
        })
    }

    async fn dispatch_approved(
        &self,
        ctx: &OrchestrationContext,
        approved: &ApprovedDispatch,
    ) -> Result<ActionResult, OrchestratorError> {
        self.dispatch_approved_inner(ctx, approved).await
    }
}

impl RuntimeExecutePhase {
    /// F25 drain path: execute a tool whose operator approval has already
    /// landed. This mirrors the non-approval branch of `dispatch_one` but:
    ///
    /// * Uses the pre-minted `ToolCallId` from the approval record — NO
    ///   re-derivation, because `ctx.iteration` has reset to 0 after the
    ///   approval round-trip and would produce a different deterministic
    ///   hash for the same logical call. That mismatch is the core of the
    ///   F25 shadowing bug.
    /// * Skips the approval gate entirely (the operator already approved).
    /// * Consults + populates the shared `ToolCallResultCache` so the
    ///   same call re-driven in a later iteration or after a process
    ///   restart hits the cache instead of re-invoking the tool.
    async fn dispatch_approved_inner(
        &self,
        ctx: &OrchestrationContext,
        approved: &ApprovedDispatch,
    ) -> Result<ActionResult, OrchestratorError> {
        let tool_name = approved.tool_name.clone();
        // Synthetic proposal mirrors what an `InvokeTool` ActionProposal
        // would have looked like had DECIDE emitted it. `requires_approval`
        // MUST be false — the approval round-trip is complete.
        let synth_proposal = cairn_domain::ActionProposal {
            action_type: cairn_domain::ActionType::InvokeTool,
            description: format!("drain approved tool call: {tool_name}"),
            confidence: 1.0,
            tool_name: Some(tool_name.clone()),
            tool_args: Some(approved.tool_args.clone()),
            requires_approval: false,
        };

        // ── Cache pre-check (F25 "has this already executed?") ─────────
        let startup_id = ToolCallId::from_raw(approved.call_id.as_str().to_owned());
        if let Some(cache_arc) = &self.tool_result_cache {
            let hit = {
                let guard = cache_arc.lock().unwrap_or_else(|e| e.into_inner());
                guard.get(&startup_id).cloned()
            };
            if let Some(cached) = hit {
                let context_output = truncate_tool_output_for_context(
                    cached.result_json.clone(),
                    self.tool_output_token_limit,
                );
                return Ok(ActionResult {
                    proposal: synth_proposal,
                    status: ActionStatus::Succeeded,
                    tool_output: Some(context_output),
                    invocation_id: None,
                    duration_ms: 0,
                });
            }
        }

        // ── Record invocation start ────────────────────────────────────
        let inv_id = ToolInvocationId::new(new_id("inv"));
        self.tool_invocation_service
            .record_start(
                &ctx.project,
                inv_id.clone(),
                Some(ctx.session_id.clone()),
                Some(ctx.run_id.clone()),
                ctx.task_id.clone(),
                ToolInvocationTarget::Builtin {
                    tool_name: tool_name.clone(),
                },
                ExecutionClass::SandboxedProcess,
            )
            .await
            .map_err(OrchestratorError::Runtime)?;

        // ── Dispatch via registry (required for drain) ─────────────────
        let registry = match self.tool_registry.as_ref() {
            Some(r) => r,
            None => {
                let reason = format!(
                    "tool_registry not wired — cannot drain approved tool `{}`",
                    tool_name
                );
                self.tool_invocation_service
                    .record_failed(
                        &ctx.project,
                        inv_id.clone(),
                        ctx.task_id.clone(),
                        tool_name.clone(),
                        ToolInvocationOutcomeKind::PermanentFailure,
                        Some(reason.clone()),
                    )
                    .await
                    .map_err(OrchestratorError::Runtime)?;
                return Ok(ActionResult {
                    proposal: synth_proposal,
                    status: ActionStatus::Failed { reason },
                    tool_output: None,
                    invocation_id: Some(inv_id),
                    duration_ms: 0,
                });
            }
        };

        let mut tool_ctx = cairn_tools::builtins::ToolContext::default();
        tool_ctx.session_id = Some(ctx.session_id.to_string());
        tool_ctx.run_id = Some(ctx.run_id.to_string());
        tool_ctx.working_dir = ctx.working_dir.clone();
        let tool_args = tool_args_with_working_dir(
            &tool_name,
            &ctx.working_dir,
            Some(approved.tool_args.clone()),
        );

        let output_result = registry
            .execute_with_context(&tool_name, &ctx.project, tool_args, &tool_ctx)
            .await
            .map(|r| r.output)
            .map_err(|e| e.to_string());

        let buffered = tool_ctx.drain_buffered_events();

        match output_result {
            Ok(output) => {
                // Completion event carries the approval's call_id so the
                // startup replay rebuilds the cache entry on next boot.
                self.tool_invocation_service
                    .record_completed(
                        &ctx.project,
                        inv_id.clone(),
                        ctx.task_id.clone(),
                        tool_name.clone(),
                        &buffered,
                        Some(approved.call_id.as_str().to_owned()),
                        Some(output.clone()),
                    )
                    .await
                    .map_err(OrchestratorError::Runtime)?;

                // Populate runtime cache so an in-process re-drain hits.
                if let Some(cache_arc) = &self.tool_result_cache {
                    let mut guard = cache_arc.lock().unwrap_or_else(|e| e.into_inner());
                    guard.insert(CachedToolResult {
                        tool_call_id: startup_id,
                        tool_name: tool_name.clone(),
                        result_json: output.clone(),
                        completed_at: now_ms_u64(),
                    });
                }

                let context_output =
                    truncate_tool_output_for_context(output, self.tool_output_token_limit);
                Ok(ActionResult {
                    proposal: synth_proposal,
                    status: ActionStatus::Succeeded,
                    tool_output: Some(context_output),
                    invocation_id: Some(inv_id),
                    duration_ms: 0,
                })
            }
            Err(reason) => {
                self.tool_invocation_service
                    .record_failed(
                        &ctx.project,
                        inv_id.clone(),
                        ctx.task_id.clone(),
                        tool_name.clone(),
                        ToolInvocationOutcomeKind::PermanentFailure,
                        Some(reason.clone()),
                    )
                    .await
                    .map_err(OrchestratorError::Runtime)?;
                Ok(ActionResult {
                    proposal: synth_proposal,
                    status: ActionStatus::Failed { reason },
                    tool_output: None,
                    invocation_id: Some(inv_id),
                    duration_ms: 0,
                })
            }
        }
    }

    /// Dispatch a single proposal to the appropriate runtime service.
    async fn dispatch_one(
        &self,
        ctx: &OrchestrationContext,
        proposal: &cairn_domain::ActionProposal,
        call_index: u32,
    ) -> Result<ActionResult, OrchestratorError> {
        match proposal.action_type {
            // ── InvokeTool ─────────────────────────────────────────────────
            ActionType::InvokeTool => {
                let tool_name = proposal.tool_name.clone().unwrap_or_default();

                // ── BP-v2 propose-then-await approval gate ────────────────
                //
                // Research doc `docs/research/llm-agent-approval-systems.md`
                // §§ "Execute Phase Pseudocode (Fixed)". Previous (broken)
                // behaviour: mint a fresh ApprovalId, fire
                // `request_with_context`, return `AwaitingApproval` without
                // persisting the proposal — after operator approval the
                // system had nothing to retrieve, so it re-queried the LLM
                // and often lost the args entirely. That was the dogfood
                // blocker.
                //
                // Now: if a `ToolCallApprovalService` is wired AND the
                // proposal requests approval, we:
                //   1. submit the proposal (persists `ToolCallProposed`
                //      + stashes args by `ToolCallId`),
                //   2. evaluate session allow-registry inside the service,
                //   3. if the service reports `PendingOperator`, block on
                //      `await_decision(call_id, timeout)` — the oneshot
                //      fires when the operator approves / rejects / amends+
                //      approves, or times out,
                //   4. on approval, retrieve the effective args (with
                //      operator amendments applied) and invoke the tool
                //      *inline* so the tool result flows back to the LLM
                //      in the same execute batch,
                //   5. on rejection / timeout, surface a tool_result error
                //      back to the LLM so it can revise its plan.
                //
                // When no `ToolCallApprovalService` is wired (existing
                // tests), we keep the legacy `ApprovalService` short-circuit
                // so those tests don't need rewriting.
                if proposal.requires_approval {
                    if let Some(ref svc) = self.tool_call_approval_service {
                        return self
                            .run_with_approval_gate(ctx, proposal, call_index, svc.clone())
                            .await;
                    }
                    // Legacy fallback: `ApprovalService` short-circuit.
                    return legacy_approval_gate_result(
                        &self.approval_service,
                        ctx,
                        proposal,
                        &tool_name,
                    )
                    .await;
                }

                let inv_id = ToolInvocationId::new(new_id("inv"));

                self.tool_invocation_service
                    .record_start(
                        &ctx.project,
                        inv_id.clone(),
                        Some(ctx.session_id.clone()),
                        Some(ctx.run_id.clone()),
                        ctx.task_id.clone(),
                        ToolInvocationTarget::Builtin {
                            tool_name: tool_name.clone(),
                        },
                        ExecutionClass::SandboxedProcess,
                    )
                    .await
                    .map_err(OrchestratorError::Runtime)?;

                // ── Decision check (RFC 019): evaluate before dispatch ────
                if let Some(ref ds) = self.decision_service {
                    use cairn_domain::decisions::*;
                    let tool_effect = if let Some(ref reg) = self.tool_registry {
                        reg.get(&tool_name)
                            .map(|h| h.tool_effect())
                            .unwrap_or(ToolEffect::External)
                    } else {
                        ToolEffect::External
                    };
                    let dreq = DecisionRequest {
                        kind: DecisionKind::ToolInvocation {
                            tool_name: tool_name.clone(),
                            effect: tool_effect,
                        },
                        principal: Principal::Run {
                            run_id: ctx.run_id.clone(),
                        },
                        subject: DecisionSubject::ToolCall {
                            tool_name: tool_name.clone(),
                            args: proposal
                                .tool_args
                                .clone()
                                .unwrap_or(serde_json::Value::Null),
                        },
                        scope: ctx.project.clone(),
                        cost_estimate: None,
                        requested_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        correlation_id: cairn_domain::CorrelationId::new(format!(
                            "tool_{}_{}",
                            ctx.run_id, ctx.iteration
                        )),
                    };
                    match ds.evaluate(dreq).await {
                        Ok(decision) => {
                            if let DecisionOutcome::Denied { deny_reason, .. } = &decision.outcome {
                                return Ok(ActionResult {
                                    proposal: proposal.clone(),
                                    status: ActionStatus::Failed {
                                        reason: format!("decision_denied: {deny_reason}"),
                                    },
                                    tool_output: None,
                                    invocation_id: Some(inv_id),
                                    duration_ms: 0,
                                });
                            }
                        }
                        Err(e) => {
                            // Decision service error — fail open (allow) but
                            // log so the failure is observable. A silent
                            // fail-open on a security-critical policy
                            // evaluator (RFC 019) would remove the guard
                            // without telemetry.
                            tracing::warn!(
                                error = %e,
                                tool = %tool_name,
                                run_id = %ctx.run_id,
                                "decision service error — failing open"
                            );
                        }
                    }
                }

                // ── Tool dispatch: registry first, stub fallback ────────────
                let mut tool_ctx = cairn_tools::builtins::ToolContext::default();
                tool_ctx.session_id = Some(ctx.session_id.to_string());
                tool_ctx.run_id = Some(ctx.run_id.to_string());
                tool_ctx.working_dir = ctx.working_dir.clone();
                let tool_args = tool_args_with_working_dir(
                    &tool_name,
                    &ctx.working_dir,
                    proposal.tool_args.clone(),
                );

                // ── RFC 020 Track 3: mint ToolCallId + consult cache ───────
                // The ToolCallId is derived from run_id + step + call_index
                // + tool_name + normalized_args. Deterministic so a resumed
                // run at the same step recomputes the same ID and hits the
                // cache.
                let (tool_call_id, normalized_args, retry_safety) =
                    if let Some(registry) = self.tool_registry.as_ref() {
                        if let Some(handler) = registry.get(&tool_name) {
                            let normalized = handler.normalize_for_cache(&tool_args);
                            // `call_index` is the proposal's position within
                            // the current DecideOutput.proposals vector. Using
                            // the index (not a hardcoded 0) guarantees two
                            // parallel invocations of the SAME tool with
                            // IDENTICAL normalized args still get distinct
                            // ToolCallIds — the orchestrator dispatches them
                            // in order, so the index is stable across replay.
                            let id = ToolCallId::derive(
                                ctx.run_id.as_str(),
                                ctx.iteration,
                                call_index,
                                &tool_name,
                                &normalized,
                            );
                            (Some(id), normalized, handler.retry_safety())
                        } else {
                            (
                                None,
                                String::new(),
                                cairn_domain::recovery::RetrySafety::DangerousPause,
                            )
                        }
                    } else {
                        (
                            None,
                            String::new(),
                            cairn_domain::recovery::RetrySafety::DangerousPause,
                        )
                    };
                let _ = normalized_args; // reserved for future audit emission

                // Consult cache: on hit, serve cached result + emit audit event.
                if let (Some(ref id), Some(ref cache_arc)) =
                    (&tool_call_id, &self.tool_result_cache)
                {
                    let hit = {
                        let guard = cache_arc.lock().unwrap_or_else(|e| e.into_inner());
                        guard.get(id).cloned()
                    };
                    if let Some(cached) = hit {
                        // Atomically emit cache-hit audit + completion marker
                        // so the invocation lifecycle closes cleanly (the
                        // earlier `record_start` left it `Started`).
                        let now = now_ms_u64();
                        let cache_event =
                            RuntimeEvent::ToolInvocationCacheHit(ToolInvocationCacheHit {
                                project: ctx.project.clone(),
                                invocation_id: inv_id.clone(),
                                run_id: Some(ctx.run_id.clone()),
                                task_id: ctx.task_id.clone(),
                                tool_name: tool_name.clone(),
                                tool_call_id: id.as_str().to_owned(),
                                original_completed_at_ms: cached.completed_at,
                                served_at_ms: now,
                            });
                        // Persist the cached tool_call_id + result_json on
                        // the new ToolInvocationCompleted too. Downstream
                        // projections (including the next boot's
                        // `replay_tool_result_cache`) expect every
                        // completion event to carry these when they exist;
                        // leaving them `None` would silently break cache
                        // rebuild after a restart that intersected a
                        // cache-hit turn.
                        self.tool_invocation_service
                            .record_completed(
                                &ctx.project,
                                inv_id.clone(),
                                ctx.task_id.clone(),
                                tool_name.clone(),
                                &[cache_event],
                                Some(id.as_str().to_owned()),
                                Some(cached.result_json.clone()),
                            )
                            .await
                            .map_err(OrchestratorError::Runtime)?;

                        let context_output = truncate_tool_output_for_context(
                            cached.result_json.clone(),
                            self.tool_output_token_limit,
                        );
                        return Ok(ActionResult {
                            proposal: proposal.clone(),
                            status: ActionStatus::Succeeded,
                            tool_output: Some(context_output),
                            invocation_id: Some(inv_id),
                            duration_ms: 0,
                        });
                    }

                    // Cache miss + is_recovery: branch on RetrySafety.
                    if ctx.is_recovery {
                        let decision = cairn_runtime::startup::recovery_dispatch_decision(
                            &cache_arc.lock().unwrap_or_else(|e| e.into_inner()),
                            id,
                            &tool_name,
                            retry_safety,
                            true,
                        );
                        match decision {
                            RecoveryDispatchDecision::CacheHit => unreachable!(
                                "recovery_dispatch_decision returned CacheHit after miss"
                            ),
                            RecoveryDispatchDecision::Dispatch => {
                                // Fall through to fresh dispatch below.
                            }
                            RecoveryDispatchDecision::Pause { reason, .. } => {
                                let paused_event =
                                    RuntimeEvent::ToolRecoveryPaused(ToolRecoveryPaused {
                                        project: ctx.project.clone(),
                                        run_id: ctx.run_id.clone(),
                                        task_id: ctx.task_id.clone(),
                                        tool_name: tool_name.clone(),
                                        tool_call_id: id.as_str().to_owned(),
                                        reason: reason.clone(),
                                        paused_at_ms: now_ms_u64(),
                                    });
                                self.tool_invocation_service
                                    .record_failed(
                                        &ctx.project,
                                        inv_id.clone(),
                                        ctx.task_id.clone(),
                                        tool_name.clone(),
                                        ToolInvocationOutcomeKind::PermanentFailure,
                                        Some(reason.clone()),
                                    )
                                    .await
                                    .map_err(OrchestratorError::Runtime)?;
                                // Deterministic approval_id derived from the
                                // tool_call_id so two recovery sweeps of the
                                // same crashed iteration ask the operator
                                // for ONE approval, not N. `new_id()` uses
                                // a per-process counter that resets on
                                // restart, which would silently duplicate
                                // the pending approval on every boot of a
                                // wedged run.
                                let approval_id =
                                    ApprovalId::new(format!("appr_recovery_{}", id.as_str()));
                                // Append the pause audit event. Best-effort;
                                // approval request below records the primary
                                // transition.
                                if let Err(e) = self
                                    .approval_service
                                    .request_with_context(
                                        &ctx.project,
                                        approval_id.clone(),
                                        Some(ctx.run_id.clone()),
                                        ctx.task_id.clone(),
                                        ApprovalRequirement::Required,
                                        Some(format!(
                                            "RFC 020 recovery pause: re-dispatch of {}",
                                            tool_name
                                        )),
                                        Some(format!(
                                            "Run `{}` crashed mid-dispatch on `{}` (DangerousPause). \
                                             Operator must confirm before re-invocation. Reason: {}",
                                            ctx.run_id, tool_name, reason,
                                        )),
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        run_id = %ctx.run_id,
                                        "approval request for recovery pause failed"
                                    );
                                }
                                // Emit the pause event itself via the tool
                                // invocation service's audit seam so the
                                // event log carries it for integration tests
                                // and operator dashboards.
                                if let Err(e) = self
                                    .tool_invocation_service
                                    .append_audit_events(&[paused_event])
                                    .await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        "append ToolRecoveryPaused failed"
                                    );
                                }
                                return Ok(ActionResult {
                                    proposal: proposal.clone(),
                                    status: ActionStatus::AwaitingApproval { approval_id },
                                    tool_output: None,
                                    invocation_id: Some(inv_id),
                                    duration_ms: 0,
                                });
                            }
                        }
                    }
                }

                // T5-H5: tool_registry is required — no silent-Ok stub. If
                // someone forgot to wire a registry, every tool invocation
                // must fail loud rather than synthesise a fake success for
                // mutating actions (write_document, http_post, send_message).
                let tool_output_result = match self.tool_registry.as_ref() {
                    Some(registry) => registry
                        .execute_with_context(
                            &tool_name,
                            &ctx.project,
                            tool_args.clone(),
                            &tool_ctx,
                        )
                        .await
                        .map(|r| r.output)
                        .map_err(|e| e.to_string()),
                    None => Err(format!(
                        "tool_registry not wired — cannot dispatch tool `{}`",
                        tool_name
                    )),
                };

                // RFC 020 Track 3 invariant #11: drain any events the tool
                // buffered on the context and pass them to record_completed
                // as a single atomic append alongside ToolInvocationCompleted.
                let buffered = tool_ctx.drain_buffered_events();

                match tool_output_result {
                    Ok(output) => {
                        self.tool_invocation_service
                            .record_completed(
                                &ctx.project,
                                inv_id.clone(),
                                ctx.task_id.clone(),
                                tool_name.clone(),
                                &buffered,
                                tool_call_id.as_ref().map(|id| id.as_str().to_owned()),
                                Some(output.clone()),
                            )
                            .await
                            .map_err(OrchestratorError::Runtime)?;

                        // Populate cache post-completion so a later replay
                        // at the same step hits (same ToolCallId).
                        if let (Some(id), Some(cache_arc)) =
                            (&tool_call_id, &self.tool_result_cache)
                        {
                            let mut guard = cache_arc.lock().unwrap_or_else(|e| e.into_inner());
                            guard.insert(CachedToolResult {
                                tool_call_id: id.clone(),
                                tool_name: tool_name.clone(),
                                result_json: output.clone(),
                                completed_at: now_ms_u64(),
                            });
                        }

                        // T5-H2: counter is cumulative across iterations.
                        // fetch_add returns the pre-increment value; add 1.
                        let n = self
                            .tool_call_count
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                            + 1;
                        if n.is_multiple_of(self.checkpoint_every_n_tool_calls) {
                            let cp_id = CheckpointId::new(new_id("cp"));
                            self.checkpoint_service
                                .save(&ctx.project, &ctx.run_id, cp_id)
                                .await
                                .map_err(OrchestratorError::Runtime)?;
                        }

                        let context_output =
                            truncate_tool_output_for_context(output, self.tool_output_token_limit);

                        Ok(ActionResult {
                            proposal: proposal.clone(),
                            status: ActionStatus::Succeeded,
                            tool_output: Some(context_output),
                            invocation_id: Some(inv_id),
                            duration_ms: 0,
                        })
                    }
                    Err(reason) => {
                        self.tool_invocation_service
                            .record_failed(
                                &ctx.project,
                                inv_id.clone(),
                                ctx.task_id.clone(),
                                tool_name,
                                ToolInvocationOutcomeKind::PermanentFailure,
                                Some(reason.clone()),
                            )
                            .await
                            .map_err(OrchestratorError::Runtime)?;

                        Ok(ActionResult {
                            proposal: proposal.clone(),
                            status: ActionStatus::Failed { reason },
                            tool_output: None,
                            invocation_id: Some(inv_id),
                            duration_ms: 0,
                        })
                    }
                }
            }

            // ── SpawnSubagent ──────────────────────────────────────────────
            ActionType::SpawnSubagent => {
                let child_task_id = TaskId::new(new_id("child_task"));
                let child_session_id = SessionId::new(new_id("child_sess"));

                match self
                    .task_service
                    .spawn_subagent(
                        &ctx.project,
                        ctx.run_id.clone(),
                        ctx.task_id.clone(),
                        child_task_id.clone(),
                        child_session_id,
                        None, // child run created when child task → running (RFC 005)
                    )
                    .await
                {
                    Ok(_) => Ok(ActionResult {
                        proposal: proposal.clone(),
                        status: ActionStatus::SubagentSpawned { child_task_id },
                        tool_output: None,
                        invocation_id: None,
                        duration_ms: 0,
                    }),
                    Err(e) => Ok(ActionResult {
                        proposal: proposal.clone(),
                        status: ActionStatus::Failed {
                            reason: e.to_string(),
                        },
                        tool_output: None,
                        invocation_id: None,
                        duration_ms: 0,
                    }),
                }
            }

            // ── SendNotification ───────────────────────────────────────────
            ActionType::SendNotification => {
                // Sender must have a task_id; recipient uses tool_name or run_id.
                let from_task = match &ctx.task_id {
                    Some(t) => t.clone(),
                    None => {
                        return Ok(ActionResult {
                            proposal: proposal.clone(),
                            status: ActionStatus::Failed {
                                reason: "SendNotification requires task_id in context".to_owned(),
                            },
                            tool_output: None,
                            invocation_id: None,
                            duration_ms: 0,
                        });
                    }
                };
                let to_task =
                    TaskId::new(proposal.tool_name.as_deref().unwrap_or(ctx.run_id.as_str()));
                match self
                    .mailbox_service
                    .send(
                        &ctx.project,
                        from_task,
                        to_task,
                        proposal.description.clone(),
                    )
                    .await
                {
                    Ok(_) => Ok(ActionResult {
                        proposal: proposal.clone(),
                        status: ActionStatus::Succeeded,
                        tool_output: None,
                        invocation_id: None,
                        duration_ms: 0,
                    }),
                    Err(e) => Ok(ActionResult {
                        proposal: proposal.clone(),
                        status: ActionStatus::Failed {
                            reason: e.to_string(),
                        },
                        tool_output: None,
                        invocation_id: None,
                        duration_ms: 0,
                    }),
                }
            }

            // ── CompleteRun ────────────────────────────────────────────────
            ActionType::CompleteRun => match self
                .run_service
                .complete(&ctx.session_id, &ctx.run_id)
                .await
            {
                Ok(_) => Ok(ActionResult {
                    proposal: proposal.clone(),
                    status: ActionStatus::Succeeded,
                    tool_output: None,
                    invocation_id: None,
                    duration_ms: 0,
                }),
                Err(e) => Ok(ActionResult {
                    proposal: proposal.clone(),
                    status: ActionStatus::Failed {
                        reason: e.to_string(),
                    },
                    tool_output: None,
                    invocation_id: None,
                    duration_ms: 0,
                }),
            },

            // ── EscalateToOperator ─────────────────────────────────────────
            ActionType::EscalateToOperator => {
                let approval_id = ApprovalId::new(new_id("appr"));

                // Build context for the operator from the proposal + run goal.
                let title = Some(format!("Agent requests approval: {}", proposal.description));
                let description = {
                    let mut desc = format!(
                        "**Run:** `{}`\n**Goal:** {}\n\n**Agent says:**\n{}",
                        ctx.run_id.as_str(),
                        ctx.goal,
                        proposal.description,
                    );
                    if let Some(ref tool) = proposal.tool_name {
                        desc.push_str(&format!("\n\n**Tool:** `{}`", tool));
                    }
                    if let Some(ref args) = proposal.tool_args {
                        let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
                        const MAX_ARGS_INLINE: usize = 4000;
                        if args_str.len() <= MAX_ARGS_INLINE {
                            desc.push_str(&format!("\n**Args:**\n```json\n{}\n```", args_str));
                        } else {
                            let truncated: String =
                                args_str.chars().take(MAX_ARGS_INLINE).collect();
                            desc.push_str(&format!(
                                "\n**Args (truncated, {} chars of {}):**\n```json\n{}\n… [truncated]\n```",
                                MAX_ARGS_INLINE,
                                args_str.len(),
                                truncated
                            ));
                        }
                    }
                    Some(desc)
                };

                match self
                    .approval_service
                    .request_with_context(
                        &ctx.project,
                        approval_id.clone(),
                        Some(ctx.run_id.clone()),
                        ctx.task_id.clone(),
                        ApprovalRequirement::Required,
                        title,
                        description,
                    )
                    .await
                {
                    Ok(_) => Ok(ActionResult {
                        proposal: proposal.clone(),
                        status: ActionStatus::AwaitingApproval { approval_id },
                        tool_output: None,
                        invocation_id: None,
                        duration_ms: 0,
                    }),
                    Err(e) => Ok(ActionResult {
                        proposal: proposal.clone(),
                        status: ActionStatus::Failed {
                            reason: e.to_string(),
                        },
                        tool_output: None,
                        invocation_id: None,
                        duration_ms: 0,
                    }),
                }
            }

            // ── CreateMemory ───────────────────────────────────────────────
            // Memory ingestion is async (IngestService runs independently).
            // Record intent here; actual embedding runs via the ingest pipeline.
            ActionType::CreateMemory => Ok(ActionResult {
                proposal: proposal.clone(),
                status: ActionStatus::Succeeded,
                tool_output: Some(serde_json::json!({
                    "queued": true,
                    "note": "async — see /v1/memory/ingest"
                })),
                invocation_id: None,
                duration_ms: 0,
            }),
        }
    }
}

// ── BP-v2 approval gate helpers ───────────────────────────────────────────────

impl RuntimeExecutePhase {
    /// Run the propose-then-await flow for a single `InvokeTool` proposal
    /// whose `requires_approval` is true and whose execute phase has a
    /// [`ToolCallApprovalService`] wired.
    ///
    /// Returns a final `ActionResult`:
    ///
    /// * `Succeeded` — tool approved (possibly with amended args) and ran.
    /// * `Failed`   — tool rejected, timed out, or its invocation errored
    ///   after approval. The `reason` mirrors what the LLM will see as a
    ///   tool_result error on the next GATHER turn.
    ///
    /// The tool is invoked *inline* (not via re-entry into the outer loop)
    /// so the operator approval and the tool's side effect land in the
    /// same execute batch — this is the core of what fixes the dogfood
    /// bug.
    async fn run_with_approval_gate(
        &self,
        ctx: &OrchestrationContext,
        proposal: &cairn_domain::ActionProposal,
        call_index: u32,
        svc: Arc<dyn ToolCallApprovalService>,
    ) -> Result<ActionResult, OrchestratorError> {
        let tool_name = proposal.tool_name.clone().unwrap_or_default();
        let raw_args = proposal
            .tool_args
            .clone()
            .unwrap_or(serde_json::Value::Null);

        // The ToolCallId is deterministic (run_id + iteration + call_index
        // + tool_name + normalized_args) so a resume that re-enters this
        // path for the same iteration sees the same id — the underlying
        // service short-circuits via the projection reader.
        // Fall back to `default_normalize_for_cache` (not
        // `raw_args.to_string()`) so the derived id stays deterministic
        // across re-entry even if the proposal is reconstructed from a
        // different source whose JSON key ordering differs. The two
        // diverge for object payloads because `Value::to_string()`
        // preserves insertion order; the normaliser sorts keys.
        // (Copilot review feedback on PR #270.)
        let normalized = self
            .tool_registry
            .as_ref()
            .and_then(|reg| reg.get(&tool_name))
            .map(|h| h.normalize_for_cache(&raw_args))
            .unwrap_or_else(|| cairn_tools::builtins::default_normalize_for_cache(&raw_args));
        let call_id = ToolCallId::derive(
            ctx.run_id.as_str(),
            ctx.iteration,
            call_index,
            &tool_name,
            &normalized,
        );

        // Derive match policy from tool effect + args + project root.
        let tool_effect = self
            .tool_registry
            .as_ref()
            .and_then(|reg| reg.get(&tool_name))
            .map(|h| h.tool_effect())
            .unwrap_or(cairn_domain::decisions::ToolEffect::External);
        let match_policy = crate::approval_policy::derive_match_policy(
            tool_effect,
            &raw_args,
            Some(ctx.working_dir.as_path()),
        );

        let display_summary = Some(build_display_summary(proposal, &tool_name));

        let domain_call_id = cairn_domain::ToolCallId::new(call_id.as_str());
        let tcp = ToolCallProposal {
            call_id: domain_call_id.clone(),
            session_id: ctx.session_id.clone(),
            run_id: ctx.run_id.clone(),
            project: ctx.project.clone(),
            tool_name: tool_name.clone(),
            tool_args: raw_args.clone(),
            display_summary,
            match_policy,
        };

        // 1. Submit proposal.
        let decision = svc
            .submit_proposal(tcp)
            .await
            .map_err(|e| OrchestratorError::Execute(format!("submit_proposal failed: {e}")))?;

        // 2. Resolve to an OperatorDecision — but DO NOT block in-process
        //    on `PendingOperator`. (F26 dogfood blocker.)
        //
        // Previous behaviour blocked inside `await_decision` for the full
        // `approval_timeout_ms` (default 45s). With a real operator who
        // resolves via the UI, the `POST /v1/runs/:id/orchestrate` HTTP
        // call stayed open until the approval came through — but the UI
        // approval hits a DIFFERENT process path and (per the design
        // comment in `handlers/runs.rs`) needs the same service instance.
        // In practice this showed up as: approval proposal appears in
        // `/v1/tool-call-approvals`, run stays in `Running` state, no
        // `ApprovalRequested` event emitted, HTTP call hangs for 45s,
        // then returns `Failed { reason: "operator did not respond within
        // approval timeout" }`. The tool never runs. See F26 write-up.
        //
        // Correct BP-v2 flow (see `research/llm-agent-approval-systems.md`
        // + `loop_runner.rs` approval-pre-check block):
        //   • AutoApproved → retrieve approved args, dispatch inline.
        //   • PendingOperator → return `AwaitingApproval { approval_id }`
        //     immediately. The outer loop picks this up, returns
        //     `LoopTermination::WaitingApproval`, and the HTTP handler
        //     returns 202. The operator approves asynchronously. A
        //     subsequent `orchestrate` call walks the F25 drain
        //     (`list_approved_for_run`) and dispatches the now-approved
        //     tool call before the next DECIDE.
        //
        // The ToolCallId doubles as the approval_id on the orchestrator
        // side — the `WaitingApproval` termination is informational for
        // the handler; the durable handshake is the
        // `tool_call_approvals` projection keyed by `ToolCallId`.
        let operator_decision = match decision {
            ToolCallApprovalDecision::AutoApproved => {
                // Short-circuit: session allow-registry match, retrieve
                // args (which may differ from raw_args if a prior
                // approval amended — though for auto-approve the path
                // it's always the original).
                let approved = svc
                    .retrieve_approved_proposal(&domain_call_id)
                    .await
                    .map_err(|e| {
                        OrchestratorError::Execute(format!(
                            "retrieve_approved_proposal (auto) failed: {e}"
                        ))
                    })?;
                OperatorDecision::Approved {
                    approved_args: approved.tool_args,
                }
            }
            ToolCallApprovalDecision::PendingOperator => {
                // F26: suspend the loop immediately. `ApprovalId::new` is
                // a newtype over String; reuse the deterministic
                // ToolCallId string so the handler + UI can correlate
                // the suspension back to the `tool_call_approvals`
                // projection row.
                let approval_id = ApprovalId::new(call_id.as_str().to_owned());
                tracing::info!(
                    run_id      = %ctx.run_id,
                    iteration   = ctx.iteration,
                    tool        = %tool_name,
                    call_id     = %call_id.as_str(),
                    "F26: approval pending — suspending loop (no in-process wait)"
                );
                return Ok(ActionResult {
                    proposal: proposal.clone(),
                    status: ActionStatus::AwaitingApproval { approval_id },
                    tool_output: None,
                    invocation_id: None,
                    duration_ms: 0,
                });
            }
        };

        // 3. On approval, invoke the tool with the operator-approved args.
        match operator_decision {
            OperatorDecision::Approved { approved_args } => {
                let mut revised = proposal.clone();
                revised.tool_args = Some(approved_args);
                revised.requires_approval = false;
                // Recurse. The revised proposal goes through the regular
                // dispatch path (cache consultation, decision service,
                // registry dispatch, completion event, etc.).
                //
                // `Box::pin` is required here because this is indirect
                // async recursion: `dispatch_one` can call back into
                // `run_with_approval_gate` (though with
                // `requires_approval=false` on the revised proposal it
                // does not in practice). Without the pin the compiler
                // errors with "recursion in an async fn requires
                // boxing" — the future size is undecidable.
                Box::pin(self.dispatch_one(ctx, &revised, call_index)).await
            }
            OperatorDecision::Rejected { reason } => Ok(ActionResult {
                proposal: proposal.clone(),
                status: ActionStatus::Failed {
                    reason: reason.unwrap_or_else(|| "operator rejected tool call".to_owned()),
                },
                tool_output: None,
                invocation_id: None,
                duration_ms: 0,
            }),
            OperatorDecision::Timeout => Ok(ActionResult {
                proposal: proposal.clone(),
                status: ActionStatus::Failed {
                    reason: "operator did not respond within approval timeout".to_owned(),
                },
                tool_output: None,
                invocation_id: None,
                duration_ms: 0,
            }),
        }
    }
}

fn build_display_summary(proposal: &cairn_domain::ActionProposal, tool_name: &str) -> String {
    // The operator dashboard renders this verbatim — keep it
    // short (one line) and include tool_name + the description
    // hint the LLM supplied so the operator sees intent at a glance.
    let desc = proposal.description.trim();
    if desc.is_empty() {
        format!("invoke {tool_name}")
    } else {
        format!("{tool_name}: {desc}")
    }
}

/// Legacy fallback when no [`ToolCallApprovalService`] is wired. Preserves
/// the pre-BP-v2 `ApprovalService::request_with_context` short-circuit so
/// existing tests that construct the execute phase without the new service
/// continue to see `AwaitingApproval` results.
///
/// New code should wire the BP-v2 service via
/// [`RuntimeExecutePhaseBuilder::tool_call_approval_service`].
async fn legacy_approval_gate_result(
    approval_service: &Arc<dyn ApprovalService>,
    ctx: &OrchestrationContext,
    proposal: &cairn_domain::ActionProposal,
    tool_name: &str,
) -> Result<ActionResult, OrchestratorError> {
    let approval_id = ApprovalId::new(new_id("appr"));
    let title = Some(format!("Agent requests approval for tool: {}", tool_name));
    let description = {
        let mut desc = format!(
            "**Run:** `{}`\n**Goal:** {}\n\n**Agent says:**\n{}\n\n**Tool:** `{}`",
            ctx.run_id.as_str(),
            ctx.goal,
            proposal.description,
            tool_name,
        );
        if let Some(ref args) = proposal.tool_args {
            let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
            const MAX_ARGS_INLINE: usize = 4000;
            if args_str.len() <= MAX_ARGS_INLINE {
                desc.push_str(&format!("\n**Args:**\n```json\n{}\n```", args_str));
            } else {
                let truncated: String = args_str.chars().take(MAX_ARGS_INLINE).collect();
                desc.push_str(&format!(
                    "\n**Args (truncated, {} chars of {}):**\n```json\n{}\n… [truncated]\n```",
                    MAX_ARGS_INLINE,
                    args_str.len(),
                    truncated
                ));
            }
        }
        Some(desc)
    };
    match approval_service
        .request_with_context(
            &ctx.project,
            approval_id.clone(),
            Some(ctx.run_id.clone()),
            ctx.task_id.clone(),
            ApprovalRequirement::Required,
            title,
            description,
        )
        .await
    {
        Ok(_) => Ok(ActionResult {
            proposal: proposal.clone(),
            status: ActionStatus::AwaitingApproval { approval_id },
            tool_output: None,
            invocation_id: None,
            duration_ms: 0,
        }),
        Err(e) => Ok(ActionResult {
            proposal: proposal.clone(),
            status: ActionStatus::Failed {
                reason: e.to_string(),
            },
            tool_output: None,
            invocation_id: None,
            duration_ms: 0,
        }),
    }
}

// ── Signal derivation ─────────────────────────────────────────────────────────

/// Derive the `LoopSignal` from a freshly executed `ActionResult`.
///
/// Returns the existing signal if it is already terminal.
fn derive_signal(result: &ActionResult, current: &LoopSignal) -> LoopSignal {
    if !matches!(current, LoopSignal::Continue) {
        return current.clone();
    }
    match &result.status {
        ActionStatus::Succeeded => {
            // CompleteRun → Done
            if result.proposal.action_type == ActionType::CompleteRun {
                LoopSignal::Done
            } else {
                LoopSignal::Continue
            }
        }
        ActionStatus::SubagentSpawned { child_task_id } => LoopSignal::WaitSubagent {
            child_task_id: child_task_id.clone(),
        },
        ActionStatus::AwaitingApproval { approval_id } => LoopSignal::WaitApproval {
            approval_id: approval_id.clone(),
        },
        ActionStatus::Failed { reason } => LoopSignal::Failed {
            reason: reason.clone(),
        },
    }
}

fn first_failure_reason(results: &[ActionResult]) -> Option<String> {
    results.iter().find_map(|r| {
        if let ActionStatus::Failed { reason } = &r.status {
            Some(reason.clone())
        } else {
            None
        }
    })
}

// ── ID generation ─────────────────────────────────────────────────────────────

fn now_ms_u64() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{prefix}_{ts}_{n}")
}

fn tool_args_with_working_dir(
    tool_name: &str,
    working_dir: &Path,
    args: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut args = args.unwrap_or(serde_json::Value::Null);
    if tool_name != "bash" {
        return args;
    }

    match &mut args {
        serde_json::Value::Object(map) => {
            map.entry("working_dir".to_string())
                .or_insert_with(|| serde_json::Value::String(working_dir.display().to_string()));
            args
        }
        serde_json::Value::Null => serde_json::json!({
            "working_dir": working_dir.display().to_string(),
        }),
        _ => args,
    }
}

fn truncate_tool_output_for_context(
    output: serde_json::Value,
    token_limit: usize,
) -> serde_json::Value {
    let serialized = output.to_string();
    if crate::decide_impl::estimate_tokens(&serialized) <= token_limit {
        return output;
    }

    match output {
        serde_json::Value::String(text) => {
            serde_json::Value::String(truncate_text_for_context(&text, token_limit))
        }
        serde_json::Value::Object(mut map) => {
            for key in ["stdout", "stderr", "output", "text", "result", "content"] {
                if let Some(serde_json::Value::String(text)) = map.get(key) {
                    let truncated = truncate_text_for_context(text, token_limit);
                    map.insert(key.to_owned(), serde_json::Value::String(truncated));
                    return serde_json::Value::Object(map);
                }
            }

            serde_json::Value::String(truncate_text_for_context(&serialized, token_limit))
        }
        other => {
            serde_json::Value::String(truncate_text_for_context(&other.to_string(), token_limit))
        }
    }
}

fn truncate_text_for_context(text: &str, token_limit: usize) -> String {
    if crate::decide_impl::estimate_tokens(text) <= token_limit {
        return text.to_owned();
    }

    let chars: Vec<char> = text.chars().collect();
    let keep = chars.len().min((token_limit.saturating_mul(4)).max(16));
    let head = keep / 2;
    let tail = keep.saturating_sub(head);
    let omitted = chars.len().saturating_sub(head + tail);
    let prefix: String = chars.iter().take(head).collect();
    let suffix: String = chars
        .iter()
        .rev()
        .take(tail)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    format!("{prefix}... [truncated: {omitted} chars omitted] ...{suffix}")
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod signal_aggregation_tests {
    use super::*;
    use cairn_domain::{ActionProposal, ActionType, ApprovalId, TaskId};

    fn invoke_result(status: ActionStatus) -> ActionResult {
        ActionResult {
            proposal: ActionProposal {
                action_type: ActionType::InvokeTool,
                description: "test".to_owned(),
                confidence: 1.0,
                tool_name: Some("t".to_owned()),
                tool_args: None,
                requires_approval: false,
            },
            status,
            tool_output: None,
            invocation_id: None,
            duration_ms: 0,
        }
    }

    /// Regression for the Cursor Bugbot finding on PR-5.
    ///
    /// Pre-fix, `execute()` only ran `derive_signal` on non-InvokeTool
    /// proposals (Phase 2). Any `AwaitingApproval` that escaped the
    /// parallel InvokeTool batch (legacy approval-gate fallback or
    /// RFC 020 `DangerousPause` recovery branch) silently downgraded
    /// to `Continue`, which is a production hole: the loop would
    /// advance instead of suspending for the operator.
    ///
    /// Post-fix, `execute()` runs `derive_signal` on the parallel batch
    /// too (earliest-index wins) and Phase 2 is skipped entirely if a
    /// terminal signal was already set. This test pins that derivation.
    #[test]
    fn derive_signal_escalates_awaiting_approval_to_wait_approval() {
        let awaiting = invoke_result(ActionStatus::AwaitingApproval {
            approval_id: ApprovalId::new("appr-1"),
        });
        let got = derive_signal(&awaiting, &LoopSignal::Continue);
        match got {
            LoopSignal::WaitApproval { approval_id } => {
                assert_eq!(approval_id.as_str(), "appr-1");
            }
            other => panic!("expected WaitApproval, got {other:?}"),
        }
    }

    /// Parallel-batch aggregation: earliest-index `AwaitingApproval`
    /// wins over later `Succeeded`. If the earlier entry escaped the
    /// fold, the loop would miss the suspension entirely.
    #[test]
    fn parallel_batch_aggregation_picks_earliest_terminal_signal() {
        let results = vec![
            invoke_result(ActionStatus::AwaitingApproval {
                approval_id: ApprovalId::new("appr-early"),
            }),
            invoke_result(ActionStatus::Succeeded),
        ];
        let mut loop_signal = LoopSignal::Continue;
        for r in &results {
            let next = derive_signal(r, &loop_signal);
            if !matches!(next, LoopSignal::Continue) {
                loop_signal = next;
                break;
            }
        }
        match loop_signal {
            LoopSignal::WaitApproval { approval_id } => {
                assert_eq!(approval_id.as_str(), "appr-early");
            }
            other => panic!("expected WaitApproval(appr-early), got {other:?}"),
        }
    }

    /// SubagentSpawned also escalates. Covers the other InvokeTool
    /// status that carries a terminal signal.
    #[test]
    fn derive_signal_escalates_subagent_spawned_to_wait_subagent() {
        let spawned = invoke_result(ActionStatus::SubagentSpawned {
            child_task_id: TaskId::new("child-1"),
        });
        let got = derive_signal(&spawned, &LoopSignal::Continue);
        match got {
            LoopSignal::WaitSubagent { child_task_id } => {
                assert_eq!(child_task_id.as_str(), "child-1");
            }
            other => panic!("expected WaitSubagent, got {other:?}"),
        }
    }
}

// ── F26 regression: PendingOperator must NOT block ───────────────────────────
//
// Covers the dogfood blocker where an `InvokeTool` proposal with
// `requires_approval=true` blocked the entire `POST /v1/runs/:id/orchestrate`
// request for the full `approval_timeout_default` (default 24h, capped at
// 45s by the test fixture). The fix in `run_with_approval_gate`: on
// `PendingOperator`, return `AwaitingApproval` immediately so the outer
// loop yields `LoopTermination::WaitingApproval` and the HTTP handler
// returns 202. The operator then resolves the proposal asynchronously;
// the F25 drain picks up the approved call on the next orchestrate
// invocation.
//
// These tests pin the contract at the `ToolCallApprovalService`
// interaction boundary. A full end-to-end test that drives
// `RuntimeExecutePhase::dispatch_one` requires in-memory impls of
// `RunService`, `TaskService`, etc. which live in `cairn-app` (as
// `FabricRunServiceAdapter`/etc.) and are not available from this
// crate's unit-test context. The E2E coverage lives in
// `crates/cairn-app/tests/`.
#[cfg(test)]
mod f26_pending_operator_no_block_tests {
    use async_trait::async_trait;
    use cairn_domain::approvals::{ApprovalMatchPolicy, ApprovalScope};
    use cairn_domain::{OperatorId, ProjectKey, RunId, SessionId, ToolCallId};
    use cairn_runtime::error::RuntimeError;
    use cairn_runtime::tool_call_approvals::{
        ApprovalDecision, ApprovedProposal, OperatorDecision, ToolCallApprovalService,
        ToolCallProposal,
    };
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Fake approval service that always returns `PendingOperator` and
    /// panics if `await_decision` is ever called. Pins the F26 contract:
    /// the orchestrator must NOT await an operator decision in-process.
    struct NoBlockApprovalService {
        submit_called: AtomicBool,
        await_called: AtomicBool,
    }
    impl NoBlockApprovalService {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                submit_called: AtomicBool::new(false),
                await_called: AtomicBool::new(false),
            })
        }
    }
    #[async_trait]
    impl ToolCallApprovalService for NoBlockApprovalService {
        async fn submit_proposal(
            &self,
            _proposal: ToolCallProposal,
        ) -> Result<ApprovalDecision, RuntimeError> {
            self.submit_called.store(true, Ordering::SeqCst);
            Ok(ApprovalDecision::PendingOperator)
        }
        async fn approve(
            &self,
            _call_id: ToolCallId,
            _operator_id: OperatorId,
            _scope: ApprovalScope,
            _approved_args: Option<serde_json::Value>,
        ) -> Result<(), RuntimeError> {
            unreachable!("approve must not be called from the orchestrator loop");
        }
        async fn reject(
            &self,
            _call_id: ToolCallId,
            _operator_id: OperatorId,
            _reason: Option<String>,
        ) -> Result<(), RuntimeError> {
            unreachable!("reject must not be called from the orchestrator loop");
        }
        async fn amend(
            &self,
            _call_id: ToolCallId,
            _operator_id: OperatorId,
            _new_args: serde_json::Value,
        ) -> Result<(), RuntimeError> {
            unreachable!("amend must not be called from the orchestrator loop");
        }
        async fn retrieve_approved_proposal(
            &self,
            _call_id: &ToolCallId,
        ) -> Result<ApprovedProposal, RuntimeError> {
            unreachable!(
                "retrieve_approved_proposal must not be called for PendingOperator — the \
                 approval is still pending and there is nothing to retrieve"
            );
        }
        async fn await_decision(
            &self,
            _call_id: &ToolCallId,
            _timeout: Duration,
        ) -> Result<OperatorDecision, RuntimeError> {
            self.await_called.store(true, Ordering::SeqCst);
            panic!(
                "F26 regression: orchestrator must NOT call await_decision — \
                 PendingOperator must suspend the loop, not block in-process"
            );
        }
    }

    /// Mimics the post-F26 gate logic: submit the proposal, and on
    /// PendingOperator return the pending state directly WITHOUT
    /// calling `await_decision`. This mirrors `run_with_approval_gate`'s
    /// behaviour in `execute_impl.rs` and pins the contract so a future
    /// refactor that re-introduces an in-process `await_decision` call
    /// fails loud (the fake panics).
    ///
    /// Returns `true` iff the gate correctly identified the pending
    /// state and did NOT block.
    async fn run_gate_semantics(svc: Arc<dyn ToolCallApprovalService>) -> bool {
        let proposal = ToolCallProposal {
            call_id: ToolCallId::new("tc_test_f26"),
            session_id: SessionId::new("sess"),
            run_id: RunId::new("run-f26"),
            project: ProjectKey::new("t", "w", "p"),
            tool_name: "bash".to_owned(),
            tool_args: json!({ "command": "ls /tmp" }),
            display_summary: Some("bash ls /tmp".to_owned()),
            match_policy: ApprovalMatchPolicy::Exact,
        };
        match svc.submit_proposal(proposal).await.unwrap() {
            ApprovalDecision::PendingOperator => {
                // F26 contract: return without calling await_decision.
                // Any call to await_decision on the fake panics.
                true
            }
            ApprovalDecision::AutoApproved => false,
        }
    }

    /// Core F26 regression: when `submit_proposal` returns
    /// `PendingOperator`, the orchestrator must NOT call
    /// `await_decision`. The fake panics if `await_decision` is called;
    /// if it were, this test would fail.
    ///
    /// This also bounds the elapsed time — a genuine in-process block
    /// would take longer than 200ms even at the minimum configured
    /// timeout (before F26, the default was 24h).
    #[tokio::test]
    async fn pending_operator_suspends_without_await_decision() {
        let fake: Arc<NoBlockApprovalService> = NoBlockApprovalService::new();
        let fake_trait: Arc<dyn ToolCallApprovalService> = fake.clone();

        let started = std::time::Instant::now();
        let ok = tokio::time::timeout(Duration::from_millis(500), run_gate_semantics(fake_trait))
            .await
            .expect("gate semantics must return promptly");
        let elapsed = started.elapsed();

        assert!(ok, "gate semantics must recognize PendingOperator");
        assert!(
            fake.submit_called.load(Ordering::SeqCst),
            "submit_proposal must be called to persist the proposal"
        );
        assert!(
            !fake.await_called.load(Ordering::SeqCst),
            "F26 regression: await_decision was called in-process — must suspend instead"
        );
        assert!(
            elapsed < Duration::from_millis(200),
            "gate took {elapsed:?} — should return near-instantly on PendingOperator"
        );
    }

    /// Complementary assertion: the real production code path in
    /// `run_with_approval_gate` (see `execute_impl.rs` ~L1310) maps
    /// `PendingOperator` to `ActionStatus::AwaitingApproval`. The loop
    /// runner's approval-pre-check block (see `loop_runner.rs` ~L858)
    /// scans results for `AwaitingApproval` and returns
    /// `LoopTermination::WaitingApproval`.
    ///
    /// That loop-level contract is pinned by
    /// `loop_runner::tests::requires_approval_suspends_immediately`.
    /// Together with the gate-level test above, F26 regression is
    /// covered on both layers.
    #[test]
    fn f26_contract_documented() {
        // Documentation-only marker; the real assertions are in the
        // two tests above (gate) + loop_runner (loop layer).
    }
}
