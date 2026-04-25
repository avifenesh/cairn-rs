//! F38 regression: when the LLM spends several iterations on
//! non-terminal tool calls, the orchestrator must nudge it toward
//! `complete_run` — and the synthetic `complete_run` schema must appear
//! FIRST in the tools array so the provider's sampler cannot keep
//! burying it.
//!
//! # Context
//!
//! F36 (PR #297) added a native `complete_run` function tool schema.
//! Dogfood v5 task-1 (2026-04-25, real GLM-4.7) proved the schema alone
//! is not enough:
//!
//! ```text
//! Task 1: "Summarize the key design principles of Minecraft creative
//!          mode in three bullet points."
//!   → max_iterations_reached after 8 iterations
//!   → tool calls: memory_search × 4, graph_query × 1,
//!                 notify_operator × 3, glob × 1, grep × 1,
//!                 complete_run × 0
//! ```
//!
//! The model saw the schema on every turn and still picked introspection
//! tools. Root cause hypothesis: GLM-4.7's tool-use fine-tuning biases
//! heavily toward research tools once the loop is running, and an
//! agressively-worded description in the schema is not enough to
//! override that prior.
//!
//! # Fix (F38)
//!
//! `crates/cairn-orchestrator/src/decide_impl.rs`:
//!   1. `complete_run` is injected at index 0 of the tools array, not
//!      appended at the end. This exploits provider sampler weighting on
//!      tool order without changing any contract.
//!   2. On iteration 3+ (0-indexed) the user message receives a
//!      `## STOP — FINAL DIRECTIVE` suffix whenever the run's step
//!      history contains no `complete_run` call. A fresh user-turn
//!      directive beats the frozen system prompt on recency and breaks
//!      GLM-4.7 out of its introspection streak.
//!
//! # This test
//!
//! Stands up a mock OpenAI-compatible provider that emulates the
//! dogfood-v5 failure mode: on the first three turns it returns a
//! `memory_search` tool_call (letting the orchestrator loop happily). On
//! the fourth turn the mock inspects the inbound user message for the
//! F38 directive, verifies it's present, and responds with a native
//! `complete_run` tool_call that terminates the run.
//!
//! Assertions:
//!   1. The run completes (NOT `max_iterations_reached`).
//!   2. The provider received AT LEAST four turns but also terminated
//!      before the 8-iteration cap.
//!   3. On iteration 4 the directive suffix WAS in the user message.
//!   4. `complete_run` sits at index 0 in the tools array on every turn.
//!   5. The final answer propagates through to the run's `summary`.
//!
//! A second test covers the pure predicate + descriptor invariants so
//! a future soft rewrite of the nudge wording or a re-ordering of the
//! tools array is caught at unit-test speed.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

const MOCK_MODEL: &str = "openrouter/f38-stuck-loop-break";
const FINAL_ANSWER: &str = "- Blocks at will\n- Flight enabled\n- No survival damage";
const EXPECTED_DIRECTIVE_HEADER: &str = "## STOP — FINAL DIRECTIVE";

#[derive(Clone)]
struct MockState {
    hits: Arc<AtomicUsize>,
    captured_tools: Arc<Mutex<Vec<Value>>>,
    captured_user_messages: Arc<Mutex<Vec<String>>>,
}

/// Spawn a mock OpenAI-compatible provider that replays the dogfood-v5
/// task-1 failure mode: introspection tool calls for iterations 0..=2,
/// then `complete_run` on iteration 3. The fourth turn is also the
/// iteration at which F38's directive suffix starts firing, so the
/// handler verifies the directive is actually present before returning
/// a terminal response — otherwise the test would pass even if the
/// nudge were never wired in.
async fn spawn_mock() -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<Value>>>, Arc<Mutex<Vec<String>>>)
{
    let state = MockState {
        hits: Arc::new(AtomicUsize::new(0)),
        captured_tools: Arc::new(Mutex::new(Vec::new())),
        captured_user_messages: Arc::new(Mutex::new(Vec::new())),
    };
    let hits = state.hits.clone();
    let captured_tools = state.captured_tools.clone();
    let captured_user = state.captured_user_messages.clone();

    async fn chat_handler(
        State(state): State<MockState>,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        let turn = state.hits.fetch_add(1, Ordering::SeqCst);

        if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
            state
                .captured_tools
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(Value::Array(tools.clone()));
        }

        // Snapshot the user-role content for every turn so the test can
        // assert the F38 directive appears on iteration 3+.
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            let user_content = messages
                .iter()
                .rev()
                .find_map(|m| {
                    if m.get("role").and_then(|r| r.as_str()) == Some("user") {
                        m.get("content").and_then(|c| c.as_str()).map(str::to_owned)
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            state
                .captured_user_messages
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(user_content);
        }

        // Turns 0..=2 (iterations 0, 1, 2): echo the dogfood-v5 failure
        // by calling `memory_search`. The orchestrator loop will execute
        // it, fail or succeed idempotently, and come back for the next
        // iteration.
        if turn <= 2 {
            return (
                StatusCode::OK,
                Json(json!({
                    "id": format!("mock-f38-turn-{turn}"),
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": format!("call_memsearch_{turn}"),
                                "type": "function",
                                "function": {
                                    "name": "memory_search",
                                    "arguments": json!({
                                        "query": "minecraft creative mode",
                                        "mode": "hybrid"
                                    }).to_string(),
                                }
                            }],
                        },
                        "finish_reason": "tool_calls",
                    }],
                    "usage": {
                        "prompt_tokens": 120,
                        "completion_tokens": 10,
                        "total_tokens": 130,
                    }
                })),
            );
        }

        // Turn 3 (iteration 3): the F38 directive must be present in the
        // user message. If it is, we terminate via `complete_run`. If it
        // isn't, the test assertion below catches it because the
        // captured user message won't contain the header.
        (
            StatusCode::OK,
            Json(json!({
                "id": "mock-f38-terminal",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_complete_1",
                            "type": "function",
                            "function": {
                                "name": "complete_run",
                                "arguments": json!({ "final_answer": FINAL_ANSWER }).to_string(),
                            }
                        }],
                    },
                    "finish_reason": "tool_calls",
                }],
                "usage": {
                    "prompt_tokens": 180,
                    "completion_tokens": 40,
                    "total_tokens": 220,
                }
            })),
        )
    }

    let app = Router::new()
        .route("/chat/completions", post(chat_handler))
        .route("/v1/chat/completions", post(chat_handler))
        .route(
            "/v1/models",
            get(|| async { Json(json!({ "data": [{"id": MOCK_MODEL}] })) }),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    (format!("http://{addr}"), hits, captured_tools, captured_user)
}

/// Provision credential + connection + defaults + session + run, then
/// POST /orchestrate. Mirrors the F36 test harness so the two tests
/// fail in comparable places when something unrelated breaks.
async fn setup_and_orchestrate(
    h: &LiveHarness,
    mock_url: &str,
    goal: &str,
    max_iterations: u32,
) -> (u16, Value) {
    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_f38_{suffix}");
    let session_id = format!("sess_f38_{suffix}");
    let run_id = format!("run_f38_{suffix}");

    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-f38-{suffix}"),
        }))
        .send()
        .await
        .expect("credential reaches server");
    assert_eq!(r.status().as_u16(), 201);
    let credential_id = r
        .json::<Value>()
        .await
        .expect("credential json")
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_owned();

    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "provider_connection_id": connection_id,
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": [MOCK_MODEL],
            "credential_id": credential_id,
            "endpoint_url": mock_url,
        }))
        .send()
        .await
        .expect("connection reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "connection: {}",
        r.text().await.unwrap_or_default()
    );

    for key in ["generate_model", "brain_model"] {
        let r = h
            .client()
            .put(format!(
                "{}/v1/settings/defaults/system/system/{}",
                h.base_url, key,
            ))
            .bearer_auth(&h.admin_token)
            .json(&json!({ "value": MOCK_MODEL }))
            .send()
            .await
            .expect("defaults reach server");
        assert_eq!(r.status().as_u16(), 200);
    }

    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "session_id": session_id,
        }))
        .send()
        .await
        .expect("session reaches server");
    assert_eq!(r.status().as_u16(), 201);

    let r = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "session_id": session_id,
            "run_id": run_id,
        }))
        .send()
        .await
        .expect("run reaches server");
    assert_eq!(r.status().as_u16(), 201);

    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id,))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": goal,
            "max_iterations": max_iterations,
        }))
        .send()
        .await
        .expect("orchestrate reaches server");
    let status = r.status().as_u16();
    let body: Value = r.json().await.unwrap_or(Value::Null);
    (status, body)
}

/// F38 primary regression: after three introspection-only iterations
/// the orchestrator breaks the loop by nudging the model onto
/// `complete_run`, and the nudge carries the load-bearing directive.
#[tokio::test]
async fn stuck_introspection_loop_recovers_on_iteration_three() {
    let h = LiveHarness::setup().await;
    let (mock_url, hits, captured_tools, captured_user) = spawn_mock().await;

    // Use the dogfood-v5 max_iterations so a regression surfaces as an
    // 8-turn failure (identical to the real bug) rather than truncating
    // early.
    let (status, body) = setup_and_orchestrate(
        &h,
        &mock_url,
        "Summarize the key design principles of Minecraft creative mode in three bullet points.",
        8,
    )
    .await;

    assert_eq!(status, 200, "orchestrate should return 200; body={body}");

    let termination = body
        .get("termination")
        .and_then(|t| t.as_str())
        .unwrap_or("<missing>");
    assert_ne!(
        termination, "max_iterations_reached",
        "F38: orchestrator must break an introspection-only loop via the \
         iteration-3 nudge. Full body: {body}",
    );

    // Exactly four provider turns: three introspection rounds + the
    // nudged terminal round. >4 means the nudge failed to land and the
    // loop kept running; <4 means the mock terminated early (shouldn't
    // happen unless someone flipped the iteration threshold).
    let n = hits.load(Ordering::SeqCst);
    assert_eq!(
        n, 4,
        "F38: expected exactly 4 LLM calls (3 introspection + 1 nudged \
         terminal); got {n}. Full body: {body}",
    );

    // The F38 directive must have landed in the user message on turn 4
    // (index 3) — not before, not after that would matter. We assert
    // the header string because it's the stable part of the suffix.
    let user_msgs = captured_user
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    assert_eq!(
        user_msgs.len(),
        4,
        "F38: expected four captured user messages; got {}",
        user_msgs.len()
    );
    for (i, msg) in user_msgs.iter().enumerate().take(3) {
        assert!(
            !msg.contains(EXPECTED_DIRECTIVE_HEADER),
            "F38: iteration {i} must NOT carry the stuck-loop directive \
             (threshold is 3). Got:\n{msg}"
        );
    }
    assert!(
        user_msgs[3].contains(EXPECTED_DIRECTIVE_HEADER),
        "F38: iteration 3 MUST carry the stuck-loop directive. Got:\n{}",
        user_msgs[3]
    );

    // `complete_run` must sit at index 0 in the tools array on every
    // turn. F38 reorders the tool_defs so the schema sits first rather
    // than last.
    let tools_snapshots = captured_tools
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    assert!(
        !tools_snapshots.is_empty(),
        "mock provider must have received a tools array on turn 1"
    );
    for (i, snapshot) in tools_snapshots.iter().enumerate() {
        let arr = snapshot
            .as_array()
            .unwrap_or_else(|| panic!("turn {i}: tools must be an array"));
        assert!(
            !arr.is_empty(),
            "turn {i}: tools array must not be empty — F38 relies on \
             complete_run being injected at index 0"
        );
        let first_name = arr[0]
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("<missing>");
        assert_eq!(
            first_name, "complete_run",
            "F38: `complete_run` must be at index 0 of the tools array \
             on every turn (turn {i}). Got: {first_name}"
        );
    }

    // Terminal answer must reach the run summary. Mirrors F36's
    // forbearance: the fabric might accept the terminal call and
    // surface it as `termination=completed`, in which case `summary`
    // carries `final_answer`. We only assert when that path fires.
    if termination == "completed" {
        assert_eq!(
            body.get("summary").and_then(|s| s.as_str()),
            Some(FINAL_ANSWER),
            "F38: run summary must carry `final_answer` verbatim so the \
             caller actually sees a response; body={body}",
        );
    }
}

/// F38 unit-level guard on the predicate + nudge wording. Keeps the
/// integration test above from being the only tripwire — a future
/// rewrite that softens the directive or flips the threshold would
/// otherwise only surface as a multi-minute subprocess failure.
#[test]
fn stuck_nudge_predicate_and_wording_match_contract() {
    use cairn_orchestrator::context::StepSummary;
    use cairn_orchestrator::decide_impl_test_hooks::{
        should_inject_stuck_nudge_for_tests, stuck_iteration_threshold_for_tests,
        stuck_nudge_suffix_for_tests,
    };

    let threshold = stuck_iteration_threshold_for_tests();
    assert_eq!(
        threshold, 3,
        "F38: threshold MUST stay at 3 — dogfood-v5 proved 4+ \
         iterations are needed to catch the GLM-4.7 introspection loop \
         while still letting healthy multi-step runs finish."
    );

    let make_step = |kind: &str| StepSummary {
        iteration: 0,
        action_kind: kind.to_owned(),
        summary: "x".to_owned(),
        succeeded: true,
    };

    // Below threshold with non-terminal history → no nudge.
    assert!(!should_inject_stuck_nudge_for_tests(
        0,
        &[make_step("invoke_tool")]
    ));
    assert!(!should_inject_stuck_nudge_for_tests(
        2,
        &[
            make_step("invoke_tool"),
            make_step("invoke_tool"),
            make_step("invoke_tool"),
        ]
    ));

    // At threshold with non-terminal history → nudge.
    assert!(should_inject_stuck_nudge_for_tests(
        3,
        &[
            make_step("invoke_tool"),
            make_step("invoke_tool"),
            make_step("invoke_tool"),
        ]
    ));

    // At threshold BUT a previous step was `complete_run` → no nudge
    // (defensive: we never want to double-terminate or re-prompt after
    // the model already agreed to finish).
    assert!(!should_inject_stuck_nudge_for_tests(
        4,
        &[
            make_step("invoke_tool"),
            make_step("complete_run"),
            make_step("invoke_tool"),
        ]
    ));

    // At threshold but EMPTY history (e.g. resume-mid-run edge case) →
    // no nudge, because there's nothing to nudge against.
    assert!(!should_inject_stuck_nudge_for_tests(3, &[]));

    // Wording: the header must be the exact literal the integration
    // test greps for, and the suffix must urge `complete_run` by name.
    let suffix = stuck_nudge_suffix_for_tests();
    assert!(
        suffix.contains(EXPECTED_DIRECTIVE_HEADER),
        "F38: suffix MUST include the stable header the integration \
         test pins on: {suffix}"
    );
    assert!(
        suffix.contains("complete_run"),
        "F38: suffix MUST name `complete_run` so the model can map the \
         directive back to the schema: {suffix}"
    );
    // Anti-soft-rewrite guard. If someone replaces "MUST" with
    // "should", GLM-4.7 will happily ignore the directive again.
    assert!(
        suffix.contains("MUST"),
        "F38: suffix MUST retain the uppercase imperative — softer \
         wording did not stop the dogfood-v5 failure: {suffix}"
    );

    // Gemini review on #300 flagged that threshold=3 might nudge
    // legitimate multi-step runs early. The mitigation is the
    // "call complete_run with what's missing" escape hatch — guard
    // that wording stays put so a future soft rewrite can't remove
    // the pressure valve and turn the nudge into a hard trap.
    let lower = suffix.to_lowercase();
    assert!(
        lower.contains("information is missing") || lower.contains("what is missing"),
        "F38: suffix MUST retain the escape-hatch wording so legitimate \
         multi-step runs can explain the gap instead of being truncated: \
         {suffix}"
    );
}

/// F38 Plan-mode guard (Gemini PR #300 HIGH). Plan mode terminates by
/// emitting a `<proposed_plan>` block, not by calling `complete_run`.
/// Forcing `complete_run` via the stuck-loop nudge would short-circuit
/// the planning contract. The decide phase must gate the nudge off in
/// `RunMode::Plan` regardless of iteration count or step history.
#[tokio::test]
async fn plan_mode_never_receives_stuck_nudge() {
    use cairn_domain::providers::{
        GenerationProvider, GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
    };
    use cairn_orchestrator::context::{GatherOutput, OrchestrationContext, StepSummary};
    use cairn_orchestrator::{DecidePhase, LlmDecidePhase};
    use std::path::PathBuf;

    // Capture the user-role message the orchestrator sends on a Plan
    // mode iteration well past the stuck-loop threshold. If the guard
    // from the Gemini review holds, the directive must not appear.
    struct CapturingProvider {
        captured: Arc<Mutex<String>>,
    }
    #[async_trait::async_trait]
    impl GenerationProvider for CapturingProvider {
        async fn generate(
            &self,
            _model: &str,
            messages: Vec<Value>,
            _settings: &ProviderBindingSettings,
            _tools: &[Value],
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            let user = messages
                .iter()
                .rev()
                .find_map(|m| {
                    if m.get("role").and_then(|r| r.as_str()) == Some("user") {
                        m.get("content").and_then(|c| c.as_str()).map(str::to_owned)
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            *self.captured.lock().unwrap_or_else(|p| p.into_inner()) = user;
            Ok(GenerationResponse {
                text: "[]".to_owned(),
                input_tokens: Some(10),
                output_tokens: Some(2),
                model_id: "test-model".to_owned(),
                tool_calls: vec![],
                finish_reason: Some("stop".to_owned()),
            })
        }
    }

    let captured = Arc::new(Mutex::new(String::new()));
    let phase = LlmDecidePhase::new(
        Arc::new(CapturingProvider {
            captured: captured.clone(),
        }),
        "test-model",
    );

    // Construct a Plan-mode context deep past the stuck threshold
    // with a history of non-`complete_run` steps — exactly the shape
    // that would trip the nudge in Direct/Execute modes.
    let ctx = OrchestrationContext {
        project: cairn_domain::ProjectKey::new("t", "w", "p"),
        session_id: cairn_domain::SessionId::new("sess-f38-plan"),
        run_id: cairn_domain::RunId::new("run-f38-plan"),
        task_id: None,
        iteration: 5,
        goal: "Propose a plan for X".to_owned(),
        agent_type: "planner".to_owned(),
        run_started_at_ms: 0,
        working_dir: PathBuf::from("/tmp"),
        run_mode: cairn_domain::decisions::RunMode::Plan,
        discovered_tool_names: vec![],
        step_history: (0..5)
            .map(|i| StepSummary {
                iteration: i,
                action_kind: "invoke_tool".to_owned(),
                summary: format!("introspect {i}"),
                succeeded: true,
            })
            .collect(),
        is_recovery: false,
        approval_timeout: None,
    };
    let gather = GatherOutput {
        step_history: ctx.step_history.clone(),
        ..Default::default()
    };

    phase.decide(&ctx, &gather).await.expect("decide ok");

    let user = captured.lock().unwrap_or_else(|p| p.into_inner()).clone();
    assert!(
        !user.contains(EXPECTED_DIRECTIVE_HEADER),
        "F38 Plan-mode guard: directive must NOT appear in Plan runs \
         even when iteration + history would otherwise trigger the \
         nudge. Got user message:\n{user}"
    );
}
