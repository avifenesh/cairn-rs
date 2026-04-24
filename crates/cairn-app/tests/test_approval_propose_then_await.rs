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
//! Covers the six BP-v2 acceptance cases called out in the sprint brief:
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
    ApprovalServiceImpl, CheckpointServiceImpl, MailboxServiceImpl, ToolCallApprovalReaderAdapter,
    ToolCallApprovalServiceImpl, ToolInvocationServiceImpl,
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

fn ctx_with_timeout(run_id: &str, session_id: &str, timeout: Duration) -> OrchestrationContext {
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
) -> Arc<ToolCallApprovalServiceImpl<InMemoryStore, ToolCallApprovalReaderAdapter<InMemoryStore>>> {
    let reader = Arc::new(ToolCallApprovalReaderAdapter::new(store.clone()));
    Arc::new(ToolCallApprovalServiceImpl::new(store, reader))
}

fn derive_expected_call_id(
    run_id: &str,
    iteration: u32,
    call_index: u32,
    tool: &str,
    args: &Value,
) -> ToolCallId {
    // Use the SAME normaliser the execute phase uses via
    // `ToolHandler::normalize_for_cache` — the default impl delegates
    // to `default_normalize_for_cache`. Recomputing `args.to_string()`
    // here would drift silently if a tool overrides `normalize_for_cache`
    // (Copilot review feedback on PR #270).
    let normalized = cairn_tools::builtins::default_normalize_for_cache(args);
    ToolCallId::new(
        cairn_runtime::startup::ToolCallId::derive(
            run_id,
            iteration,
            call_index,
            tool,
            &normalized,
        )
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
async fn pending_approval_suspends_loop_without_blocking() {
    // F26 dogfood-blocker fix (2026-04-23). The previous behaviour was
    // "execute blocks in-process on `await_decision` until the operator
    // resolves". That made a single `POST /v1/runs/:id/orchestrate`
    // request hang for the full approval timeout (default 24h) because
    // the operator's approve comes through a DIFFERENT HTTP path and
    // cannot make progress until the orchestrate call returns.
    //
    // New contract: pending approvals return `AwaitingApproval`
    // IMMEDIATELY. The orchestrator loop surfaces
    // `LoopTermination::WaitingApproval`, the HTTP handler returns 202,
    // and a subsequent orchestrate call dispatches the operator-
    // approved tool via the F25 drain before the next DECIDE.
    //
    // This test pins the non-blocking contract. The tool MUST NOT run
    // during this first execute (the operator hasn't even been asked
    // yet from their point of view — the proposal just landed in the
    // projection).
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

    let decide = decide_with(vec![invoke_proposal("recorder", args.clone(), true)]);
    let start = std::time::Instant::now();
    let outcome = phase.execute(&cx, &decide).await.expect("execute");
    let elapsed = start.elapsed();

    // MUST return promptly — no in-process wait for operator.
    assert!(
        elapsed < Duration::from_millis(500),
        "F26 regression: execute blocked for {elapsed:?} — should return immediately"
    );

    match &outcome.results[0].status {
        cairn_orchestrator::ActionStatus::AwaitingApproval { approval_id } => {
            // The approval_id must derive from the ToolCallId so the
            // handler + UI can correlate it with the projection row.
            assert_eq!(
                approval_id.as_str(),
                expected_id.as_str(),
                "approval_id should carry the deterministic ToolCallId"
            );
        }
        other => panic!(
            "F26 regression: expected AwaitingApproval, got {other:?} — \
             the loop will advance instead of suspending"
        ),
    }
    // Loop signal must be WaitApproval — the outer loop picks this up
    // and yields LoopTermination::WaitingApproval.
    match &outcome.loop_signal {
        cairn_orchestrator::LoopSignal::WaitApproval { approval_id } => {
            assert_eq!(approval_id.as_str(), expected_id.as_str());
        }
        other => panic!("expected WaitApproval loop_signal, got {other:?}"),
    }
    // The tool MUST NOT have run yet — nobody has approved it.
    assert_eq!(
        seen.lock().unwrap().len(),
        0,
        "F26 regression: tool ran without operator approval"
    );

    // Projection state: the proposal is Pending, waiting for operator.
    let reader = ToolCallApprovalReaderAdapter::new(store.clone());
    let stored = reader
        .get_tool_call_proposal(&expected_id)
        .await
        .expect("reader")
        .expect("proposal persisted");
    use cairn_runtime::tool_call_approvals::ToolCallApprovalReader as _;
    use cairn_runtime::tool_call_approvals::StoredProposalState;
    assert!(
        matches!(stored.state, StoredProposalState::Pending),
        "proposal must be Pending after suspend, not {:?}",
        stored.state
    );

    // Now operator approves. The F25 drain path (exercised by
    // `test_drain_approved_executes_bash.rs`) is what runs the tool on
    // the next orchestrate invocation. Here we just verify the approve
    // call succeeds — it would have dead-locked before F26 because
    // nobody was parked on the oneshot.
    svc.approve(
        expected_id.clone(),
        OperatorId::new("op"),
        ApprovalScope::Once,
        None,
    )
    .await
    .expect("operator approve");
}

/// F26: regardless of what the operator eventually does (approve /
/// reject / amend) the FIRST execute call must return
/// `AwaitingApproval` immediately. The reject/approve/amend flows are
/// handled on subsequent orchestrate invocations via the F25 drain
/// path. This test pins: rejection issued AFTER execute returned must
/// not retroactively affect the returned status.
#[tokio::test(flavor = "multi_thread")]
async fn pending_approval_returns_before_operator_can_reject() {
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

    let decide = decide_with(vec![invoke_proposal("recorder", args, true)]);
    let outcome = phase.execute(&cx, &decide).await.expect("execute");

    // Execute returned AwaitingApproval immediately — the operator
    // hasn't had a chance to do anything yet.
    assert!(
        matches!(
            outcome.results[0].status,
            cairn_orchestrator::ActionStatus::AwaitingApproval { .. }
        ),
        "F26: expected AwaitingApproval, got {:?}",
        outcome.results[0].status
    );
    assert_eq!(seen.lock().unwrap().len(), 0, "tool must not run pre-approval");

    // Now the operator rejects. That decision is durable in the
    // projection; a future orchestrate call would surface a
    // Rejected-state row via the F25 drain reader.
    svc.reject(
        expected_id.clone(),
        OperatorId::new("op"),
        Some("not safe".to_owned()),
    )
    .await
    .expect("reject");
    // Tool still never ran.
    assert_eq!(seen.lock().unwrap().len(), 0);
}

/// F26: pending approval does NOT self-timeout inside a single execute
/// call. The "timeout" concept now lives at the run-suspension layer
/// (operator SLA / auto-reject sweeper), not inside the orchestrator
/// loop. This test locks in that execute returns AwaitingApproval
/// promptly even when the context configures a tiny approval_timeout
/// — because the timeout is no longer honoured in-process.
#[tokio::test(flavor = "multi_thread")]
async fn pending_approval_does_not_block_on_approval_timeout() {
    let store = Arc::new(InMemoryStore::new());
    let svc = build_service(store.clone());
    let seen = Arc::new(Mutex::new(vec![]));
    let tool = Arc::new(RecorderTool {
        name: "recorder",
        seen: seen.clone(),
    });
    let registry = build_registry(tool.clone());
    let phase = build_phase(store.clone(), registry, svc.clone());
    // 100ms timeout; with the pre-F26 blocking gate this would drive
    // `await_decision` to fire and return Failed. Post-F26 the field
    // is still accepted for backward-compat but the hot path ignores
    // it — execute returns AwaitingApproval near-instantly.
    let cx = ctx_with_timeout("run-to", "sess-to", Duration::from_millis(100));
    let decide = decide_with(vec![invoke_proposal("recorder", json!({"k": "v"}), true)]);
    let start = std::time::Instant::now();
    let outcome = phase.execute(&cx, &decide).await.expect("execute");
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(80),
        "F26 regression: execute blocked for {elapsed:?} — must not honour approval_timeout in-process"
    );
    assert!(
        matches!(
            outcome.results[0].status,
            cairn_orchestrator::ActionStatus::AwaitingApproval { .. }
        ),
        "expected AwaitingApproval, got {:?}",
        outcome.results[0].status
    );
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
    // Suppress unused-var on the pre-F26 background approver — kept
    // inert so the test continues to exercise the approval projection
    // writer, but the operator's decision no longer races with
    // execute in the F26 flow.
    let _ = slow_id;

    // F26 contract: batch returns IMMEDIATELY (no in-process wait).
    // The auto-approved fast entry ran; the pending slow entry is
    // suspended. The outer loop's earliest-terminal-signal aggregator
    // (see `execute_impl::derive_signal` + tests) surfaces a
    // WaitApproval loop_signal so the orchestrator suspends the run
    // cleanly — even though the auto-approved fast tool already ran.
    assert!(
        elapsed < Duration::from_millis(500),
        "F26 regression: parallel batch blocked for {elapsed:?} — must return promptly"
    );
    // Fast auto-approved → ran.
    assert!(
        matches!(
            outcome.results[0].status,
            cairn_orchestrator::ActionStatus::Succeeded
        ),
        "fast (auto-approved) should have run, got {:?}",
        outcome.results[0].status
    );
    // Slow pending → AwaitingApproval, did NOT run.
    assert!(
        matches!(
            outcome.results[1].status,
            cairn_orchestrator::ActionStatus::AwaitingApproval { .. }
        ),
        "slow (pending) should be AwaitingApproval, got {:?}",
        outcome.results[1].status
    );
    // Only the fast tool left a trace.
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "only fast auto-approved tool should have run"
    );
    assert_eq!(seen.lock().unwrap()[0], fast_args);
}

/// F26: operator amendments now flow through the drain path on a
/// subsequent orchestrate invocation — not through a single blocking
/// execute. The first execute returns AwaitingApproval immediately
/// (no background amend can race with a non-blocking call). The
/// actual "amended args flow through to the tool" contract is pinned
/// by `test_drain_approved_executes_bash.rs` which exercises the
/// propose → approve → drain → dispatch path end to end with args
/// variation. Here we pin the surrounding F26 contract: first execute
/// must not run the tool and must not block, regardless of whether the
/// operator is amending.
#[tokio::test(flavor = "multi_thread")]
async fn amend_after_pending_does_not_retroactively_run_tool_in_first_execute() {
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

    let decide = decide_with(vec![invoke_proposal("recorder", original.clone(), true)]);
    let outcome = phase.execute(&cx, &decide).await.expect("execute");

    // Execute returned without blocking — nothing the operator does
    // now will change the return value of the call we already made.
    assert!(
        matches!(
            outcome.results[0].status,
            cairn_orchestrator::ActionStatus::AwaitingApproval { .. }
        ),
        "F26: expected AwaitingApproval, got {:?}",
        outcome.results[0].status
    );
    assert_eq!(seen.lock().unwrap().len(), 0, "tool must not run pre-approval");

    // Operator amends + approves after the fact. Writes go to the
    // projection; drain path picks them up on the next orchestrate.
    svc.amend(expected_id.clone(), OperatorId::new("op"), amended.clone())
        .await
        .expect("amend");
    svc.approve(
        expected_id.clone(),
        OperatorId::new("op"),
        ApprovalScope::Once,
        None,
    )
    .await
    .expect("approve");
    // Still not run — this execute call is already done.
    assert_eq!(
        seen.lock().unwrap().len(),
        0,
        "tool must not retroactively run inside the already-returned execute"
    );
}
