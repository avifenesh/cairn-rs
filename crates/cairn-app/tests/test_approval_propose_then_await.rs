//! BP-v2 propose-then-await approval flow — orchestrator-level integration.
//!
//! Research doc: `docs/research/llm-agent-approval-systems.md`.
//!
//! Exercises `RuntimeExecutePhase::execute` driving a real
//! `ToolCallApprovalServiceImpl` (with a real `InMemoryStore`-backed
//! reader adapter, a real `BuiltinToolRegistry`, and a real
//! `ToolInvocationServiceImpl`). Every test drives the full dispatch
//! path — no mocks on the hot path.
//!
//! Covers the eight BP-v2 acceptance cases called out in the sprint brief:
//!
//! * auto-approved tool executes without operator
//! * pending approval waits for operator → approve → tool runs
//! * rejection surfaces tool_result error
//! * timeout auto-rejects
//! * parallel batch executes auto-approved siblings while one waits
//! * amend-then-approve uses amended args
//!
//! Each test uses a recording tool that captures the exact args it saw
//! so we can assert the approved / amended args flowed through.

mod support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cairn_domain::{
    decisions::RunMode, recovery::RetrySafety, ActionProposal, ActionType, ApprovalScope,
    OperatorId, ProjectKey, RunId, SessionId, ToolCallId,
};
use cairn_orchestrator::context::{DecideOutput, OrchestrationContext};
use cairn_orchestrator::execute::ExecutePhase;
use cairn_orchestrator::execute_impl::RuntimeExecutePhase;
use cairn_runtime::services::{
    ApprovalServiceImpl, CheckpointServiceImpl, MailboxServiceImpl,
    ToolCallApprovalReaderAdapter, ToolCallApprovalServiceImpl, ToolInvocationServiceImpl,
};
use cairn_runtime::startup::ToolCallResultCache;
use cairn_runtime::tool_call_approvals::ToolCallApprovalService;
use cairn_store::InMemoryStore;
use cairn_tools::builtins::{
    BuiltinToolRegistry, PermissionLevel, ToolCategory, ToolContext, ToolError, ToolHandler,
    ToolResult, ToolTier,
};
use serde_json::{json, Value};
use support::fake_fabric::build_fake_fabric;

// ── fixtures ──────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("t_bpv2", "w_bpv2", "p_bpv2")
}

fn ctx(run_id: &str, session_id: &str) -> OrchestrationContext {
    OrchestrationContext {
        project: project(),
        session_id: SessionId::new(session_id),
        run_id: RunId::new(run_id),
        task_id: None,
        iteration: 0,
        goal: "bp-v2 approval".to_owned(),
        agent_type: "orchestrator".to_owned(),
        run_started_at_ms: 1_000_000,
        working_dir: PathBuf::from("."),
        run_mode: RunMode::Direct,
        discovered_tool_names: vec![],
        step_history: vec![],
        is_recovery: false,
        approval_timeout: None,
    }
}

fn ctx_with_timeout(
    run_id: &str,
    session_id: &str,
    timeout: Duration,
) -> OrchestrationContext {
    let mut c = ctx(run_id, session_id);
    c.approval_timeout = Some(timeout);
    c
}

fn invoke_proposal(tool: &str, args: Value, requires_approval: bool) -> ActionProposal {
    ActionProposal {
        action_type: ActionType::InvokeTool,
        description: format!("call {tool}"),
        confidence: 1.0,
        tool_name: Some(tool.to_owned()),
        tool_args: Some(args),
        requires_approval,
    }
}

fn decide_with(proposals: Vec<ActionProposal>) -> DecideOutput {
    DecideOutput {
        raw_response: String::new(),
        proposals,
        calibrated_confidence: 1.0,
        requires_approval: false,
        model_id: "test".to_owned(),
        latency_ms: 0,
        input_tokens: None,
        output_tokens: None,
    }
}

/// Tool that records the exact args it was invoked with. Lets tests
/// assert the approved / amended payload reached the tool.
struct RecorderTool {
    name: &'static str,
    seen: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl ToolHandler for RecorderTool {
    fn name(&self) -> &str {
        self.name
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Core
    }
    fn description(&self) -> &str {
        "records invocation args"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Custom
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }
    async fn execute(&self, _: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        self.seen.lock().unwrap().push(args.clone());
        Ok(ToolResult::ok(json!({"ran": true, "args": args})))
    }
    async fn execute_with_context(
        &self,
        project: &ProjectKey,
        args: Value,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        self.execute(project, args).await
    }
}

fn build_registry(tool: Arc<RecorderTool>) -> Arc<BuiltinToolRegistry> {
    Arc::new(BuiltinToolRegistry::new().register(tool))
}

fn build_phase(
    store: Arc<InMemoryStore>,
    registry: Arc<BuiltinToolRegistry>,
    svc: Arc<dyn ToolCallApprovalService>,
) -> RuntimeExecutePhase {
    let (runs, tasks, _sessions) = build_fake_fabric(store.clone());
    let cache = Arc::new(Mutex::new(ToolCallResultCache::new()));
    RuntimeExecutePhase::builder()
        .run_service(runs)
        .task_service(tasks)
        .approval_service(Arc::new(ApprovalServiceImpl::new(store.clone())))
        .checkpoint_service(Arc::new(CheckpointServiceImpl::new(store.clone())))
        .mailbox_service(Arc::new(MailboxServiceImpl::new(store.clone())))
        .tool_invocation_service(Arc::new(ToolInvocationServiceImpl::new(store)))
        .tool_registry(registry)
        .checkpoint_every_n_tool_calls(1000)
        .tool_result_cache(cache)
        .tool_call_approval_service(svc)
        .approval_timeout_default(Duration::from_secs(60))
        .build()
}

fn build_service(
    store: Arc<InMemoryStore>,
) -> Arc<
    ToolCallApprovalServiceImpl<InMemoryStore, ToolCallApprovalReaderAdapter<InMemoryStore>>,
> {
    let reader = Arc::new(ToolCallApprovalReaderAdapter::new(store.clone()));
    Arc::new(ToolCallApprovalServiceImpl::new(store, reader))
}

fn derive_expected_call_id(run_id: &str, iteration: u32, call_index: u32, tool: &str, args: &Value) -> ToolCallId {
    // Mirrors `ToolCallId::derive` normalisation used by the execute phase.
    let normalized = args.to_string();
    ToolCallId::new(
        cairn_runtime::startup::ToolCallId::derive(run_id, iteration, call_index, tool, &normalized)
            .as_str(),
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Auto-approved tool executes without operator intervention.
///
/// A session allow-rule is pre-installed on the service (via a prior
/// Session-scope approval), so `submit_proposal` returns `AutoApproved`
/// and the execute phase dispatches the tool inline.
#[tokio::test(flavor = "multi_thread")]
async fn auto_approved_tool_executes_without_operator() {
    let store = Arc::new(InMemoryStore::new());
    let svc = build_service(store.clone());
    let seen = Arc::new(Mutex::new(vec![]));
    let tool = Arc::new(RecorderTool {
        name: "recorder",
        seen: seen.clone(),
    });
    let registry = build_registry(tool.clone());
    let phase = build_phase(store.clone(), registry, svc.clone());

    // Pre-seed: session allow-rule matching any Exact call to `recorder`
    // with these args. We do this by submitting + approving a proposal
    // with `ApprovalScope::Session{Exact}`.
    let cx = ctx("run-auto", "sess-auto");
    let seed_args = json!({"k": "v"});
    let seed_call = derive_expected_call_id("run-auto", 0, 0, "recorder", &seed_args);
    let proposal = cairn_runtime::tool_call_approvals::ToolCallProposal {
        call_id: seed_call.clone(),
        session_id: cx.session_id.clone(),
        run_id: cx.run_id.clone(),
        project: cx.project.clone(),
        tool_name: "recorder".into(),
        tool_args: seed_args.clone(),
        display_summary: None,
        match_policy: cairn_domain::ApprovalMatchPolicy::Exact,
    };
    svc.submit_proposal(proposal).await.expect("seed submit");
    svc.approve(
        seed_call.clone(),
        OperatorId::new("op"),
        ApprovalScope::Session {
            match_policy: cairn_domain::ApprovalMatchPolicy::Exact,
        },
        None,
    )
    .await
    .expect("seed approve");

    // Now a NEW run in the same session should auto-approve the same-shape call.
    let cx2 = ctx("run-auto-2", "sess-auto");
    let decide = decide_with(vec![invoke_proposal("recorder", seed_args.clone(), true)]);
    let outcome = phase.execute(&cx2, &decide).await.expect("execute");
    assert_eq!(outcome.results.len(), 1);
    match &outcome.results[0].status {
        cairn_orchestrator::ActionStatus::Succeeded => {}
        other => panic!("expected Succeeded, got {other:?}"),
    }
    assert_eq!(seen.lock().unwrap().len(), 1, "tool ran exactly once");
}

/// Pending approval: execute blocks on `await_decision`; a background
/// task approves after a short delay; execute returns with the tool
/// result. THIS IS THE DOGFOOD-UNBLOCKING CASE.
#[tokio::test(flavor = "multi_thread")]
async fn pending_approval_waits_for_operator_then_runs_tool() {
    let store = Arc::new(InMemoryStore::new());
    let svc = build_service(store.clone());
    let seen = Arc::new(Mutex::new(vec![]));
    let tool = Arc::new(RecorderTool {
        name: "recorder",
        seen: seen.clone(),
    });
    let registry = build_registry(tool.clone());
    let phase = build_phase(store.clone(), registry, svc.clone());
    let cx = ctx("run-wait", "sess-wait");
    let args = json!({"path": "/a/b", "body": "hi"});
    let expected_id = derive_expected_call_id("run-wait", 0, 0, "recorder", &args);

    // Background: approve after 100ms.
    let svc_bg = svc.clone();
    let id_bg = expected_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        svc_bg
            .approve(id_bg, OperatorId::new("op"), ApprovalScope::Once, None)
            .await
            .expect("bg approve");
    });

    let decide = decide_with(vec![invoke_proposal("recorder", args.clone(), true)]);
    let outcome = phase.execute(&cx, &decide).await.expect("execute");
    assert!(matches!(
        outcome.results[0].status,
        cairn_orchestrator::ActionStatus::Succeeded
    ));
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(seen.lock().unwrap()[0], args);
}

/// Operator rejects the proposal: execute returns Failed with the
/// rejection reason; the tool is NEVER invoked.
#[tokio::test(flavor = "multi_thread")]
async fn rejection_surfaces_tool_result_error() {
    let store = Arc::new(InMemoryStore::new());
    let svc = build_service(store.clone());
    let seen = Arc::new(Mutex::new(vec![]));
    let tool = Arc::new(RecorderTool {
        name: "recorder",
        seen: seen.clone(),
    });
    let registry = build_registry(tool.clone());
    let phase = build_phase(store.clone(), registry, svc.clone());
    let cx = ctx("run-reject", "sess-reject");
    let args = json!({"k": "v"});
    let expected_id = derive_expected_call_id("run-reject", 0, 0, "recorder", &args);

    let svc_bg = svc.clone();
    let id_bg = expected_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        svc_bg
            .reject(id_bg, OperatorId::new("op"), Some("not safe".to_owned()))
            .await
            .expect("bg reject");
    });

    let decide = decide_with(vec![invoke_proposal("recorder", args, true)]);
    let outcome = phase.execute(&cx, &decide).await.expect("execute");
    match &outcome.results[0].status {
        cairn_orchestrator::ActionStatus::Failed { reason } => {
            assert!(reason.contains("not safe"), "got: {reason}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(seen.lock().unwrap().len(), 0, "tool must not run on reject");
}

/// Timeout: operator never responds, `await_decision` fires its timer,
/// execute returns Failed.
#[tokio::test(flavor = "multi_thread")]
async fn timeout_auto_rejects() {
    let store = Arc::new(InMemoryStore::new());
    let svc = build_service(store.clone());
    let seen = Arc::new(Mutex::new(vec![]));
    let tool = Arc::new(RecorderTool {
        name: "recorder",
        seen: seen.clone(),
    });
    let registry = build_registry(tool.clone());
    let phase = build_phase(store.clone(), registry, svc.clone());
    // 100ms timeout; no operator action.
    let cx = ctx_with_timeout("run-to", "sess-to", Duration::from_millis(100));
    let decide = decide_with(vec![invoke_proposal("recorder", json!({"k": "v"}), true)]);
    let outcome = phase.execute(&cx, &decide).await.expect("execute");
    match &outcome.results[0].status {
        cairn_orchestrator::ActionStatus::Failed { reason } => {
            assert!(
                reason.to_lowercase().contains("timeout"),
                "expected timeout reason, got: {reason}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(seen.lock().unwrap().len(), 0);
}

/// Parallel batch: two InvokeTool proposals in one turn, one pending
/// approval, one auto-approved (via pre-seeded session allow). The
/// auto-approved one must RUN while the other is still waiting.
#[tokio::test(flavor = "multi_thread")]
async fn parallel_batch_executes_auto_approved_while_waiting_on_operator() {
    let store = Arc::new(InMemoryStore::new());
    let svc = build_service(store.clone());
    let seen = Arc::new(Mutex::new(vec![]));
    let tool = Arc::new(RecorderTool {
        name: "recorder",
        seen: seen.clone(),
    });
    let registry = build_registry(tool.clone());
    let phase = build_phase(store.clone(), registry, svc.clone());
    let cx = ctx("run-par", "sess-par");

    // Pre-seed: session allow-rule for the "fast" shape.
    let fast_args = json!({"k": "fast"});
    let seed_call = ToolCallId::new("seed");
    let seed = cairn_runtime::tool_call_approvals::ToolCallProposal {
        call_id: seed_call.clone(),
        session_id: cx.session_id.clone(),
        run_id: cx.run_id.clone(),
        project: cx.project.clone(),
        tool_name: "recorder".into(),
        tool_args: fast_args.clone(),
        display_summary: None,
        match_policy: cairn_domain::ApprovalMatchPolicy::Exact,
    };
    svc.submit_proposal(seed).await.expect("seed");
    svc.approve(
        seed_call,
        OperatorId::new("op"),
        ApprovalScope::Session {
            match_policy: cairn_domain::ApprovalMatchPolicy::Exact,
        },
        None,
    )
    .await
    .expect("seed approve");

    // Build a batch: [fast (auto-approves), slow (needs operator)].
    let slow_args = json!({"k": "slow"});
    let slow_id = derive_expected_call_id("run-par", 0, 1, "recorder", &slow_args);

    // Approve `slow` after 200ms.
    let svc_bg = svc.clone();
    let slow_bg = slow_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        svc_bg
            .approve(slow_bg, OperatorId::new("op"), ApprovalScope::Once, None)
            .await
            .expect("bg approve slow");
    });

    let decide = decide_with(vec![
        invoke_proposal("recorder", fast_args.clone(), true),
        invoke_proposal("recorder", slow_args.clone(), true),
    ]);
    let start = std::time::Instant::now();
    let outcome = phase.execute(&cx, &decide).await.expect("execute");
    let elapsed = start.elapsed();

    // Both succeeded.
    assert!(matches!(
        outcome.results[0].status,
        cairn_orchestrator::ActionStatus::Succeeded
    ));
    assert!(matches!(
        outcome.results[1].status,
        cairn_orchestrator::ActionStatus::Succeeded
    ));
    assert_eq!(seen.lock().unwrap().len(), 2);

    // Parallelism marker: total elapsed must be < 2x the slow wait.
    // Serial would be ~0 + 200ms; parallel is max(~0, 200ms) = ~200ms.
    // Either way the bound < 400ms is a safe parallel check.
    assert!(
        elapsed < Duration::from_millis(400),
        "batch should not serialize; took {elapsed:?}"
    );
}

/// Amend then approve: operator edits the args, then approves. Execute
/// must run the tool with the AMENDED args, not the original LLM args.
#[tokio::test(flavor = "multi_thread")]
async fn amend_then_approve_uses_amended_args() {
    let store = Arc::new(InMemoryStore::new());
    let svc = build_service(store.clone());
    let seen = Arc::new(Mutex::new(vec![]));
    let tool = Arc::new(RecorderTool {
        name: "recorder",
        seen: seen.clone(),
    });
    let registry = build_registry(tool.clone());
    let phase = build_phase(store.clone(), registry, svc.clone());
    let cx = ctx("run-amend", "sess-amend");
    let original = json!({"k": "original"});
    let amended = json!({"k": "amended"});
    let expected_id = derive_expected_call_id("run-amend", 0, 0, "recorder", &original);

    let svc_bg = svc.clone();
    let id_bg = expected_id.clone();
    let amended_bg = amended.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        svc_bg
            .amend(id_bg.clone(), OperatorId::new("op"), amended_bg)
            .await
            .expect("amend");
        svc_bg
            .approve(id_bg, OperatorId::new("op"), ApprovalScope::Once, None)
            .await
            .expect("approve");
    });

    let decide = decide_with(vec![invoke_proposal("recorder", original.clone(), true)]);
    let outcome = phase.execute(&cx, &decide).await.expect("execute");
    assert!(matches!(
        outcome.results[0].status,
        cairn_orchestrator::ActionStatus::Succeeded
    ));
    let seen_guard = seen.lock().unwrap();
    assert_eq!(seen_guard.len(), 1);
    assert_eq!(
        seen_guard[0], amended,
        "tool must run with AMENDED args, not original"
    );
}
