//! F36 regression: simple prompts must terminate via native `complete_run`
//! tool calls, not burn iterations in an introspection loop.
//!
//! # Context
//!
//! F30 (PR #293, merged 2026-04-23) rewrote the DECIDE system prompt to
//! instruct the LLM to call `complete_run` on direct-answer goals. The
//! prompt was right; it didn't stick. Dogfood v4 evidence
//! (2026-04-25, real GLM-4.7) for three trivially-answerable prompts:
//!
//! ```text
//! Task 2: "Write a short Python Fibonacci function with docstring."
//!   → max_iterations_reached after 8 iterations
//!   → 9 tool calls: memory_search × 5, search_events × 1,
//!                   notify_operator × 2, complete_run × 0
//! ```
//!
//! Root cause: with native tool calling on, the LLM saw ~20 real tool
//! schemas (memory_search, bash, read, search_events, notify_operator,
//! …) and **zero schema for `complete_run`**. `complete_run` was only
//! reachable via the legacy JSON-action-array text channel. The model
//! followed the path of least resistance and kept picking tools that
//! had compelling schemas; it never emitted the plain-text JSON array
//! needed to terminate the run.
//!
//! # Fix
//!
//! `crates/cairn-orchestrator/src/decide_impl.rs` now injects a
//! synthetic `complete_run` tool definition into every DECIDE call's
//! tool list. It takes one argument (`final_answer: string`) and the
//! description is aggressively directive:
//!
//! > "Call this IMMEDIATELY when you can answer from training data or
//! > from context already in this prompt — do not call memory_search,
//! > search_events, or any other tool first on trivially-answerable
//! > prompts."
//!
//! `tool_calls_to_proposals` special-cases `complete_run`: it builds an
//! `ActionType::CompleteRun` proposal (not `InvokeTool`) with the
//! `final_answer` argument copied into `description`. The existing
//! EXECUTE / loop-runner path then terminates the run and hands the
//! answer back as the run's `summary`.
//!
//! # This test
//!
//! Stands up a mock OpenAI-compatible provider that emits a native
//! `complete_run` tool_call (NOT a JSON-array text response — that was
//! the F30 shape; F36 covers the native tool-call shape the real model
//! should actually produce). Asserts:
//!
//!   1. The orchestrate call made exactly ONE provider round-trip.
//!   2. The provider request included a `complete_run` tool schema
//!      (i.e. the synthetic descriptor was injected).
//!   3. The run did NOT terminate with `max_iterations_reached`.
//!   4. When the fabric accepts the terminal call, the `summary` field
//!      carries the final answer text back to the caller.
//!
//! A second test inspects the system prompt's synthetic-schema side:
//! the tool_defs array passed to the provider MUST include a
//! `complete_run` function definition. This is a snapshot-style guard
//! against a future refactor silently dropping the injection.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

const MOCK_MODEL: &str = "openrouter/f36-native-complete-run";
const FINAL_ANSWER: &str = "Paris.";

#[derive(Clone)]
struct MockState {
    hits: Arc<AtomicUsize>,
    captured_tools: Arc<std::sync::Mutex<Vec<Value>>>,
}

/// Spawn a mock OpenAI-compatible provider that responds with a native
/// `complete_run` tool_call on its first (and only) turn. Captures the
/// `tools` array the orchestrator sent so the test can assert the
/// synthetic `complete_run` schema was actually injected.
async fn spawn_mock() -> (String, Arc<AtomicUsize>, Arc<std::sync::Mutex<Vec<Value>>>) {
    let state = MockState {
        hits: Arc::new(AtomicUsize::new(0)),
        captured_tools: Arc::new(std::sync::Mutex::new(Vec::new())),
    };
    let hits = state.hits.clone();
    let captured = state.captured_tools.clone();

    async fn chat_handler(
        State(state): State<MockState>,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        state.hits.fetch_add(1, Ordering::SeqCst);

        // Capture the tools array so the test can assert schema injection.
        if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
            let mut slot = state
                .captured_tools
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            slot.push(Value::Array(tools.clone()));
        }

        // Respond with a native tool_call emitting `complete_run`. This is
        // the exact shape GLM-4.7 / GPT-4 / Claude would return on a
        // direct-answer prompt AFTER F36 publishes the schema.
        (
            StatusCode::OK,
            Json(json!({
                "id": "mock-f36",
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
                    "prompt_tokens": 120,
                    "completion_tokens": 12,
                    "total_tokens": 132,
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
    (format!("http://{addr}"), hits, captured)
}

/// Provision credential + provider connection + defaults + session + run,
/// then POST to `/orchestrate`. Returns the status + body.
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
    let connection_id = format!("conn_f36_{suffix}");
    let session_id = format!("sess_f36_{suffix}");
    let run_id = format!("run_f36_{suffix}");

    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-f36-{suffix}"),
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

/// F36 primary regression: a native `complete_run` tool_call on the first
/// turn must terminate the run in one iteration, with the `final_answer`
/// argument propagating to the run's `summary`.
#[tokio::test]
async fn native_complete_run_tool_call_terminates_run_in_one_iteration() {
    let h = LiveHarness::setup().await;
    let (mock_url, hits, captured_tools) = spawn_mock().await;

    let (status, body) = setup_and_orchestrate(
        &h,
        &mock_url,
        "What is the capital of France?",
        // Match the dogfood-v4 max_iterations so a regression presents
        // identically (8 hits, never terminating) rather than truncating
        // silently.
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
        "F36: orchestrator must NOT exhaust iterations when the model \
         emits a native complete_run tool_call. Full body: {body}",
    );

    // Exactly one provider round-trip: the tool_call produced a terminal
    // CompleteRun proposal, which the loop runner converts to Done.
    let n = hits.load(Ordering::SeqCst);
    assert_eq!(
        n, 1,
        "F36: expected exactly 1 LLM call; got {n}. >1 means the \
         tool_call was NOT recognised as terminal and the loop \
         continued. Full body: {body}",
    );

    // Guard: the provider received a `complete_run` schema in the tools
    // array. Without this, the mock could respond however it wanted but a
    // real model wouldn't know the tool exists. This is the load-bearing
    // F36 invariant — the descriptor injection must reach the wire.
    let tools_snapshots = captured_tools
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    assert!(
        !tools_snapshots.is_empty(),
        "mock provider must have received a tools array"
    );
    let tools_array = tools_snapshots[0]
        .as_array()
        .expect("first capture must be an array");
    let has_complete_run_schema = tools_array.iter().any(|t| {
        t.get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            == Some("complete_run")
    });
    assert!(
        has_complete_run_schema,
        "F36: `complete_run` must appear as a tool schema in the provider \
         request. Got tools: {tools_array:#?}",
    );

    // If the fabric accepted the terminal call (the common case in
    // LiveHarness), the user-facing answer must propagate. We mirror the
    // F30 test's forbearance: a downstream fabric error would surface as
    // termination="failed" with an execute-phase reason, orthogonal to
    // F36 — so only assert the summary payload when termination is
    // "completed".
    if termination == "completed" {
        assert_eq!(
            body.get("summary").and_then(|s| s.as_str()),
            Some(FINAL_ANSWER),
            "F36: run summary must carry `final_answer` verbatim so the \
             caller actually sees a response; body={body}",
        );
    }
}

/// F36 schema-injection regression: the tool_defs array passed to the
/// brain provider during DECIDE MUST include a `complete_run` function
/// definition with a `final_answer` string parameter. This is the
/// load-bearing piece of F36 — without it, the LLM has no native path
/// to terminate and dogfood-v4's 8-iteration introspection loop returns.
///
/// Unit-level assertion on `complete_run_tool_def()` so a future edit to
/// the descriptor is caught before it hits HTTP.
#[test]
fn complete_run_tool_def_has_required_shape() {
    use cairn_orchestrator::decide_impl_test_hooks::complete_run_tool_def_for_tests;

    let def = complete_run_tool_def_for_tests();

    assert_eq!(def["type"], "function", "must be an OpenAI function tool");
    assert_eq!(
        def["function"]["name"], "complete_run",
        "tool name must be complete_run"
    );

    let desc = def["function"]["description"]
        .as_str()
        .expect("description must be a string");
    // The description is the model's anchor for "when to call this".
    // Require aggressively-directive wording so a future soft rewrite
    // doesn't silently weaken the F36 nudge.
    assert!(
        desc.to_lowercase().contains("immediately"),
        "description must urge immediate use: {desc}"
    );
    assert!(
        desc.contains("final_answer"),
        "description must reference the `final_answer` argument: {desc}"
    );

    let params = &def["function"]["parameters"];
    assert_eq!(params["type"], "object", "parameters must be a JSON object");
    assert!(
        params["properties"]["final_answer"]["type"] == "string",
        "final_answer must be typed as string"
    );
    let required = params["required"]
        .as_array()
        .expect("required must be an array");
    assert!(
        required
            .iter()
            .any(|v| v.as_str() == Some("final_answer")),
        "final_answer must be a required parameter"
    );
}
