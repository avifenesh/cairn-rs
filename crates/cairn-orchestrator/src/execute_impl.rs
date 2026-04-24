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
use std::time::Duration;
use cairn_tools::builtins::BuiltinToolRegistry;
#[allow(unused_imports)]
use cairn_tools::builtins::ToolHandler;
use std::sync::Mutex;

use crate::context::{
    ActionResult, ActionStatus, DecideOutput, ExecuteOutcome, LoopSignal, OrchestrationContext,
};
use crate::error::OrchestratorError;
use crate::execute::ExecutePhase;

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
    pub fn tool_call_approval_service(
        mut self,
        svc: Arc<dyn ToolCallApprovalService>,
    ) -> Self {
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

        let mut results: Vec<Option<ActionResult>> = (0..decide.proposals.len())
            .map(|_| None)
            .collect();
        let mut loop_signal = LoopSignal::Continue;

        // ── Phase 1: parallel InvokeTool dispatch ────────────────────────
        let invoke_indices: Vec<usize> = decide
            .proposals
            .iter()
            .enumerate()
            .filter_map(|(i, p)| (p.action_type == ActionType::InvokeTool).then_some(i))
            .collect();

        if !invoke_indices.is_empty() {
            let futs = invoke_indices.iter().map(|&i| {
                let proposal = decide.proposals[i].clone();
                async move {
                    let started_at = std::time::Instant::now();
                    let mut result =
                        self.dispatch_one(ctx, &proposal, i as u32).await?;
                    result.duration_ms = started_at.elapsed().as_millis() as u64;
                    Ok::<_, OrchestratorError>((i, result))
                }
            });
            let joined = futures::future::join_all(futs).await;
            for outcome in joined {
                let (i, result) = outcome?;
                results[i] = Some(result);
            }
        }

        // ── Phase 2: sequential non-InvokeTool dispatch ──────────────────
        //
        // These carry terminal signals (CompleteRun → Done, SpawnSubagent
        // → WaitSubagent, EscalateToOperator → WaitApproval). Original
        // short-circuit semantics: first terminal signal stops the batch.
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

        // Fill any still-None slots with a synthesized Failed so downstream
        // consumers never encounter a gap — this only happens when a
        // terminal non-InvokeTool short-circuited before later non-InvokeTool
        // proposals ran. The gap is expected; mark as skipped.
        let results: Vec<ActionResult> = results
            .into_iter()
            .enumerate()
            .filter_map(|(i, r)| {
                r.or_else(|| {
                    // Proposal was skipped by a terminal short-circuit.
                    Some(ActionResult {
                        proposal: decide.proposals[i].clone(),
                        status: ActionStatus::Failed {
                            reason: "skipped: earlier proposal terminated the batch"
                                .to_owned(),
                        },
                        tool_output: None,
                        invocation_id: None,
                        duration_ms: 0,
                    })
                })
            })
            .collect();

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
}

impl RuntimeExecutePhase {
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
        let normalized = self
            .tool_registry
            .as_ref()
            .and_then(|reg| reg.get(&tool_name))
            .map(|h| h.normalize_for_cache(&raw_args))
            .unwrap_or_else(|| raw_args.to_string());
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
        let decision = svc.submit_proposal(tcp).await.map_err(|e| {
            OrchestratorError::Execute(format!("submit_proposal failed: {e}"))
        })?;

        // 2. Resolve to an OperatorDecision, awaiting if pending.
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
                let timeout = ctx
                    .approval_timeout
                    .unwrap_or(self.approval_timeout_default);
                svc.await_decision(&domain_call_id, timeout)
                    .await
                    .map_err(|e| {
                        OrchestratorError::Execute(format!("await_decision failed: {e}"))
                    })?
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
                Box::pin(self.dispatch_one(ctx, &revised, call_index)).await
            }
            OperatorDecision::Rejected { reason } => Ok(ActionResult {
                proposal: proposal.clone(),
                status: ActionStatus::Failed {
                    reason: reason
                        .unwrap_or_else(|| "operator rejected tool call".to_owned()),
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

fn build_display_summary(
    proposal: &cairn_domain::ActionProposal,
    tool_name: &str,
) -> String {
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
