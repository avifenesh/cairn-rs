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
    decisions::DecisionService,
    mailbox::MailboxService,
    services::{TaskServiceImpl, ToolInvocationService},
    ApprovalService, CheckpointService, RunService,
};
use cairn_store::InMemoryStore;
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
    task_service: Arc<TaskServiceImpl<InMemoryStore>>,
    approval_service: Arc<dyn ApprovalService>,
    checkpoint_service: Arc<dyn CheckpointService>,
    mailbox_service: Arc<dyn MailboxService>,
    tool_invocation_service: Arc<dyn ToolInvocationService>,
    /// Registered built-in tools (memory_search, memory_store, …).
    /// When `Some`, tool names are looked up here before falling back to the
    /// stub dispatcher.
    tool_registry: Option<Arc<BuiltinToolRegistry>>,
    /// Decision service for pre-dispatch policy evaluation (RFC 019).
    decision_service: Option<Arc<dyn DecisionService>>,
    /// Save a checkpoint after every N-th successful tool call (1 = every call).
    checkpoint_every_n_tool_calls: u32,
    /// Maximum size of tool output copied back into the LLM context.
    tool_output_token_limit: usize,
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
    task_service: Option<Arc<TaskServiceImpl<InMemoryStore>>>,
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
    pub fn task_service(mut self, s: Arc<TaskServiceImpl<InMemoryStore>>) -> Self {
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
        let mut tool_call_count: u32 = 0;
        let mut loop_signal = LoopSignal::Continue;

        for proposal in &decide.proposals {
            let result = self
                .dispatch_one(ctx, proposal, &mut tool_call_count)
                .await?;

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
        tool_call_count: &mut u32,
    ) -> Result<ActionResult, OrchestratorError> {
        match proposal.action_type {
            // ── InvokeTool ─────────────────────────────────────────────────
            ActionType::InvokeTool => {
                let tool_name = proposal.tool_name.clone().unwrap_or_default();
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
                                });
                            }
                        }
                        Err(_) => {
                            // Decision service error — fail open (allow).
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
                let tool_output_result = if let Some(ref registry) = self.tool_registry {
                    // Look up the tool in the built-in registry.
                    // `execute_with_context()` returns Result<ToolResult, ToolError>; map to
                    // the flat Result<Value, String> expected below.
                    registry
                        .execute_with_context(
                            &tool_name,
                            &ctx.project,
                            tool_args.clone(),
                            &tool_ctx,
                        )
                        .await
                        .map(|r| r.output)
                        .map_err(|e| e.to_string())
                } else {
                    dispatch_tool(&tool_name, Some(&tool_args))
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

                        *tool_call_count += 1;
                        if (*tool_call_count).is_multiple_of(self.checkpoint_every_n_tool_calls) {
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
                    }),
                    Err(e) => Ok(ActionResult {
                        proposal: proposal.clone(),
                        status: ActionStatus::Failed {
                            reason: e.to_string(),
                        },
                        tool_output: None,
                        invocation_id: None,
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
                    }),
                    Err(e) => Ok(ActionResult {
                        proposal: proposal.clone(),
                        status: ActionStatus::Failed {
                            reason: e.to_string(),
                        },
                        tool_output: None,
                        invocation_id: None,
                    }),
                }
            }

            // ── CompleteRun ────────────────────────────────────────────────
            ActionType::CompleteRun => match self.run_service.complete(&ctx.run_id).await {
                Ok(_) => Ok(ActionResult {
                    proposal: proposal.clone(),
                    status: ActionStatus::Succeeded,
                    tool_output: None,
                    invocation_id: None,
                }),
                Err(e) => Ok(ActionResult {
                    proposal: proposal.clone(),
                    status: ActionStatus::Failed {
                        reason: e.to_string(),
                    },
                    tool_output: None,
                    invocation_id: None,
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
                        if args_str.len() < 2000 {
                            desc.push_str(&format!("\n**Args:**\n```json\n{}\n```", args_str));
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
                    }),
                    Err(e) => Ok(ActionResult {
                        proposal: proposal.clone(),
                        status: ActionStatus::Failed {
                            reason: e.to_string(),
                        },
                        tool_output: None,
                        invocation_id: None,
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

// ── Built-in tool dispatch stub ───────────────────────────────────────────────

/// Dispatch a built-in tool call.
///
/// For v1 this returns a stub observation for known tools.  A real
/// implementation looks up a `ToolRegistry` / `PluginHost`.
fn dispatch_tool(
    tool_name: &str,
    args: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    match tool_name {
        "search_memory" | "read_document" | "write_document" | "send_message" | "list_tasks"
        | "http_get" | "http_post" => Ok(serde_json::json!({
            "tool":   tool_name,
            "args":   args,
            "result": null,
            "note":   "stub — real dispatch not yet wired"
        })),
        other => Err(format!("unknown tool: {other}")),
    }
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{
        lifecycle::RunState, EventEnvelope, EventId, EventSource, RunStateChanged, StateTransition,
    };
    use cairn_domain::{ActionProposal, ActionType, ProjectKey, RunId, SessionId};
    use cairn_runtime::{
        services::{
            ApprovalServiceImpl, CheckpointServiceImpl, MailboxServiceImpl, RunServiceImpl,
            TaskServiceImpl, ToolInvocationServiceImpl,
        },
        InMemoryServices,
    };
    use cairn_store::EventLog;
    use cairn_tools::builtins::{
        BuiltinToolRegistry, ToolError, ToolHandler, ToolResult, ToolTier,
    };
    use serde_json::Value;
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::context::{DecideOutput, LoopSignal, OrchestrationContext};

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }
    fn run_id() -> RunId {
        RunId::new("run_exec_test")
    }
    fn session_id() -> SessionId {
        SessionId::new("sess_exec_test")
    }

    /// Build all services wired to the same shared store.
    fn make_services() -> Arc<InMemoryServices> {
        Arc::new(InMemoryServices::new())
    }

    /// Build a `RuntimeExecutePhase` from a shared `InMemoryServices`.
    ///
    /// Each service is created as a NEW concrete impl sharing the same
    /// underlying `Arc<InMemoryStore>`.  This is the correct pattern:
    /// all service impls read/write the same store so changes are
    /// immediately visible across services.
    fn make_phase(svc: &Arc<InMemoryServices>) -> RuntimeExecutePhase {
        let store = svc.store.clone();
        RuntimeExecutePhase::builder()
            .run_service(Arc::new(RunServiceImpl::new(store.clone())))
            .task_service(Arc::new(TaskServiceImpl::new(store.clone())))
            .approval_service(Arc::new(ApprovalServiceImpl::new(store.clone())))
            .checkpoint_service(Arc::new(CheckpointServiceImpl::new(store.clone())))
            .mailbox_service(Arc::new(MailboxServiceImpl::new(store.clone())))
            .tool_invocation_service(Arc::new(ToolInvocationServiceImpl::new(store)))
            .checkpoint_every_n_tool_calls(1)
            .build()
    }

    fn ctx() -> OrchestrationContext {
        OrchestrationContext {
            project: project(),
            session_id: session_id(),
            run_id: run_id(),
            task_id: None,
            iteration: 0,
            goal: "test goal".to_owned(),
            agent_type: "test_agent".to_owned(),
            run_started_at_ms: 0,
            working_dir: PathBuf::from("."),
            run_mode: cairn_domain::decisions::RunMode::Direct,
            discovered_tool_names: vec![],
        }
    }

    fn decide_with(proposals: Vec<ActionProposal>) -> DecideOutput {
        DecideOutput {
            raw_response: "{}".to_owned(),
            proposals,
            calibrated_confidence: 0.9,
            requires_approval: false,
            model_id: "stub_model".to_owned(),
            latency_ms: 0,
            input_tokens: None,
            output_tokens: None,
        }
    }

    struct LargeOutputTool;

    #[async_trait::async_trait]
    impl ToolHandler for LargeOutputTool {
        fn name(&self) -> &str {
            "large_output"
        }

        fn tier(&self) -> ToolTier {
            ToolTier::Registered
        }

        fn description(&self) -> &str {
            "Return an intentionally large stdout payload."
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(&self, _: &ProjectKey, _: Value) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::ok(serde_json::json!({
                "stdout": "abcdefghij".repeat(400),
            })))
        }
    }

    /// Seed a session + a run in `Running` state so `complete`/`fail`/`approve`
    /// transitions are valid.
    async fn setup_run(svc: &Arc<InMemoryServices>) {
        svc.sessions.create(&project(), session_id()).await.unwrap();
        svc.runs
            .start(&project(), &session_id(), run_id(), None)
            .await
            .unwrap();

        // RunService::complete requires Running state.  Emit a transition event.
        svc.store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_run_start_exec_test"),
                EventSource::Runtime,
                cairn_domain::RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project(),
                    run_id: run_id(),
                    transition: StateTransition {
                        from: Some(RunState::Pending),
                        to: RunState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            )])
            .await
            .unwrap();
    }

    // ─────────────────────────────────────────────────────────────────────────
    // complete_run
    // ─────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn complete_run_emits_done_signal_and_updates_run_state() {
        let svc = make_services();
        setup_run(&svc).await;

        let outcome = make_phase(&svc)
            .execute(
                &ctx(),
                &decide_with(vec![ActionProposal::complete_run("all done", 0.95)]),
            )
            .await
            .unwrap();

        assert_eq!(outcome.loop_signal, LoopSignal::Done);
        assert_eq!(outcome.results[0].status, ActionStatus::Succeeded);

        let run = svc.runs.get(&run_id()).await.unwrap().unwrap();
        assert_eq!(run.state, cairn_domain::lifecycle::RunState::Completed);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // escalate_to_operator
    // ─────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn escalate_creates_approval_and_emits_wait_signal() {
        let svc = make_services();
        setup_run(&svc).await;

        let outcome = make_phase(&svc)
            .execute(
                &ctx(),
                &decide_with(vec![ActionProposal::escalate("need human", 0.3)]),
            )
            .await
            .unwrap();

        assert!(
            matches!(outcome.loop_signal, LoopSignal::WaitApproval { .. }),
            "expected WaitApproval, got {:?}",
            outcome.loop_signal
        );
        assert!(matches!(
            outcome.results[0].status,
            ActionStatus::AwaitingApproval { .. }
        ));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // tool call — known tool
    // ─────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn known_tool_call_succeeds_records_events_and_saves_checkpoint() {
        let svc = make_services();
        setup_run(&svc).await;

        let outcome = make_phase(&svc)
            .execute(
                &ctx(),
                &decide_with(vec![ActionProposal::invoke_tool(
                    "search_memory",
                    serde_json::json!({ "q": "rust ownership" }),
                    "search for context",
                    0.8,
                    false,
                )]),
            )
            .await
            .unwrap();

        assert_eq!(outcome.loop_signal, LoopSignal::Continue);
        assert_eq!(outcome.results[0].status, ActionStatus::Succeeded);
        assert!(
            outcome.results[0].invocation_id.is_some(),
            "tool call must record a ToolInvocationId"
        );
        assert!(
            outcome.results[0].tool_output.is_some(),
            "tool call must return an observation"
        );

        // Checkpoint must have been saved (checkpoint_every_n=1).
        let cp = svc.checkpoints.latest_for_run(&run_id()).await.unwrap();
        assert!(
            cp.is_some(),
            "checkpoint must be saved after successful tool call"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // tool call — unknown tool
    // ─────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn unknown_tool_returns_failed_status_and_failed_signal() {
        let svc = make_services();
        setup_run(&svc).await;

        let outcome = make_phase(&svc)
            .execute(
                &ctx(),
                &decide_with(vec![ActionProposal::invoke_tool(
                    "nonexistent_tool",
                    serde_json::json!({}),
                    "call unknown tool",
                    0.5,
                    false,
                )]),
            )
            .await
            .unwrap();

        assert!(
            matches!(outcome.loop_signal, LoopSignal::Failed { .. }),
            "unknown tool must produce a Failed signal"
        );
        assert!(matches!(
            outcome.results[0].status,
            ActionStatus::Failed { .. }
        ));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // spawn_subagent
    // ─────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn spawn_subagent_creates_child_task_and_emits_wait_subagent() {
        let svc = make_services();
        setup_run(&svc).await;

        let outcome = make_phase(&svc)
            .execute(
                &ctx(),
                &decide_with(vec![ActionProposal {
                    action_type: ActionType::SpawnSubagent,
                    description: "delegate to researcher".to_owned(),
                    confidence: 0.75,
                    tool_name: Some("research_agent".to_owned()),
                    tool_args: None,
                    requires_approval: false,
                }]),
            )
            .await
            .unwrap();

        assert!(
            matches!(outcome.loop_signal, LoopSignal::WaitSubagent { .. }),
            "spawn must emit WaitSubagent signal"
        );
        assert!(matches!(
            outcome.results[0].status,
            ActionStatus::SubagentSpawned { .. }
        ));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // create_memory — async no-op
    // ─────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_memory_is_a_noop_that_succeeds() {
        let svc = make_services();
        setup_run(&svc).await;

        let outcome = make_phase(&svc)
            .execute(
                &ctx(),
                &decide_with(vec![ActionProposal {
                    action_type: ActionType::CreateMemory,
                    description: "store knowledge".to_owned(),
                    confidence: 0.9,
                    tool_name: None,
                    tool_args: Some(serde_json::json!({ "content": "sky is blue" })),
                    requires_approval: false,
                }]),
            )
            .await
            .unwrap();

        assert_eq!(outcome.loop_signal, LoopSignal::Continue);
        assert_eq!(outcome.results[0].status, ActionStatus::Succeeded);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // short-circuit: complete_run stops further proposals
    // ─────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn complete_run_short_circuits_remaining_proposals() {
        let svc = make_services();
        setup_run(&svc).await;

        let outcome = make_phase(&svc)
            .execute(
                &ctx(),
                &decide_with(vec![
                    ActionProposal::invoke_tool(
                        "search_memory",
                        serde_json::json!({}),
                        "search first",
                        0.8,
                        false,
                    ),
                    ActionProposal::complete_run("done", 0.95),
                    // This third proposal must NOT be executed.
                    ActionProposal::invoke_tool(
                        "http_get",
                        serde_json::json!({}),
                        "never reached",
                        0.5,
                        false,
                    ),
                ]),
            )
            .await
            .unwrap();

        assert_eq!(outcome.loop_signal, LoopSignal::Done);
        assert_eq!(
            outcome.results.len(),
            2,
            "third proposal must be cut off after complete_run"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // checkpoint frequency: only every N-th tool call
    // ─────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn checkpoint_saved_every_n_tool_calls() {
        let svc = make_services();
        setup_run(&svc).await;

        let store = svc.store.clone();
        let phase = RuntimeExecutePhase::builder()
            .run_service(Arc::new(RunServiceImpl::new(store.clone())))
            .task_service(Arc::new(TaskServiceImpl::new(store.clone())))
            .approval_service(Arc::new(ApprovalServiceImpl::new(store.clone())))
            .checkpoint_service(Arc::new(CheckpointServiceImpl::new(store.clone())))
            .mailbox_service(Arc::new(MailboxServiceImpl::new(store.clone())))
            .tool_invocation_service(Arc::new(ToolInvocationServiceImpl::new(store)))
            .checkpoint_every_n_tool_calls(3) // save only on 1st, 4th, 7th, … call
            .build();

        // 2 tool calls → count = 2, neither is divisible by 3 → no checkpoint yet.
        phase
            .execute(
                &ctx(),
                &decide_with(vec![
                    ActionProposal::invoke_tool(
                        "search_memory",
                        serde_json::json!({}),
                        "s1",
                        0.8,
                        false,
                    ),
                    ActionProposal::invoke_tool(
                        "search_memory",
                        serde_json::json!({}),
                        "s2",
                        0.8,
                        false,
                    ),
                ]),
            )
            .await
            .unwrap();
        let cp = svc.checkpoints.latest_for_run(&run_id()).await.unwrap();
        assert!(cp.is_none(), "no checkpoint after 2 of 3 required calls");
    }

    #[tokio::test]
    async fn tool_output_is_truncated_to_context_token_limit() {
        let svc = make_services();
        setup_run(&svc).await;

        let store = svc.store.clone();
        let registry = Arc::new(BuiltinToolRegistry::new().register(Arc::new(LargeOutputTool)));
        let phase = RuntimeExecutePhase::builder()
            .run_service(Arc::new(RunServiceImpl::new(store.clone())))
            .task_service(Arc::new(TaskServiceImpl::new(store.clone())))
            .approval_service(Arc::new(ApprovalServiceImpl::new(store.clone())))
            .checkpoint_service(Arc::new(CheckpointServiceImpl::new(store.clone())))
            .mailbox_service(Arc::new(MailboxServiceImpl::new(store.clone())))
            .tool_invocation_service(Arc::new(ToolInvocationServiceImpl::new(store)))
            .tool_registry(registry)
            .tool_output_token_limit(32)
            .build();

        let outcome = phase
            .execute(
                &ctx(),
                &decide_with(vec![ActionProposal::invoke_tool(
                    "large_output",
                    serde_json::json!({}),
                    "emit a large payload",
                    0.9,
                    false,
                )]),
            )
            .await
            .unwrap();

        let stdout = outcome.results[0].tool_output.as_ref().unwrap()["stdout"]
            .as_str()
            .unwrap();
        assert!(
            stdout.contains("[truncated:"),
            "large tool output should include a truncation marker"
        );
        assert!(
            crate::decide_impl::estimate_tokens(stdout) <= 64,
            "context-facing tool output should be materially smaller after truncation"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ExecutePhase is object-safe
    // ─────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_phase_is_object_safe() {
        let svc = make_services();
        setup_run(&svc).await;

        let phase: Box<dyn ExecutePhase> = Box::new(make_phase(&svc));
        let outcome = phase
            .execute(
                &ctx(),
                &decide_with(vec![ActionProposal::complete_run("done", 1.0)]),
            )
            .await
            .unwrap();
        assert_eq!(outcome.loop_signal, LoopSignal::Done);
    }
}
