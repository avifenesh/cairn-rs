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
    ActionType, ApprovalId, CheckpointId, ExecutionClass, SessionId, TaskId, ToolInvocationId,
};
use cairn_runtime::{
    decisions::DecisionService, mailbox::MailboxService, services::ToolInvocationService,
    ApprovalService, CheckpointService, RunService, TaskService,
};
use cairn_tools::builtins::BuiltinToolRegistry;

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
        let mut results: Vec<ActionResult> = Vec::with_capacity(decide.proposals.len());
        let mut loop_signal = LoopSignal::Continue;

        for proposal in &decide.proposals {
            // Per-proposal wall-clock; `dispatch_one` initialises
            // `duration_ms: 0` and we overwrite with the real elapsed.
            let started_at = std::time::Instant::now();
            let mut result = self.dispatch_one(ctx, proposal).await?;
            result.duration_ms = started_at.elapsed().as_millis() as u64;

            // Capture any terminal loop signal from this action before pushing.
            let new_signal = derive_signal(&result, &loop_signal);

            results.push(result);

            if !matches!(new_signal, LoopSignal::Continue) {
                loop_signal = new_signal;
                break; // short-circuit: first terminal action stops the batch
            }
        }

        // If nothing set a terminal signal but any action failed, derive Failed.
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
    ) -> Result<ActionResult, OrchestratorError> {
        match proposal.action_type {
            // ── InvokeTool ─────────────────────────────────────────────────
            ActionType::InvokeTool => {
                let tool_name = proposal.tool_name.clone().unwrap_or_default();

                // T5-C1: approval gate MUST short-circuit before record_start
                // or registry.execute_with_context. A proposal with
                // requires_approval=true represents an operator-review
                // decision; running the tool first and then reporting
                // AwaitingApproval to the loop runner bypasses the gate.
                if proposal.requires_approval {
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
                            // Truncate inline rather than hiding entirely —
                            // the operator needs *some* visibility into
                            // tools like `write_document` whose payloads
                            // routinely exceed 2000 chars.
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
                    return match self
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
                    };
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

                match tool_output_result {
                    Ok(output) => {
                        self.tool_invocation_service
                            .record_completed(
                                &ctx.project,
                                inv_id.clone(),
                                ctx.task_id.clone(),
                                tool_name,
                            )
                            .await
                            .map_err(OrchestratorError::Runtime)?;

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
    if tool_name != "shell_exec" {
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
