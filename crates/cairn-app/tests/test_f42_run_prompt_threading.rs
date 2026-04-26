//! F42 regression: `POST /v1/runs` must accept an operator-supplied
//! `prompt` field and thread it into the orchestrator's LLM user
//! message as the `## Goal` section.
//!
//! # Context
//!
//! Dogfood v8 (2026-04-26, post-F41 binary) achieved the first
//! successful end-to-end completion of three runs — the pipeline
//! works. But every LLM response said:
//!
//! > "I was asked to 'execute the run objective' but no specific
//! > objective was provided."
//!
//! The POST body included a `prompt` field:
//!
//! ```json
//! {"tenant_id":"acme","workspace_id":"prod","project_id":"minecraft",
//!  "run_id":"dogfood-v8-task-1","session_id":"mc-dogfood",
//!  "prompt":"Summarize the key design principles of Minecraft ..."}
//! ```
//!
//! The server returned 201 (no 422 complaint) but the `prompt` field
//! did NOT appear on the Run projection. Serde was silently dropping
//! unknown fields on `CreateRunRequest`, and the orchestrator's
//! `OrchestrationContext.goal` fell back to the hard-coded string
//! `"Execute the run objective."` — which the LLM then dutifully
//! reported it could not fulfil.
//!
//! # Fix
//!
//! 1. Added `prompt: Option<String>` to `CreateRunRequest`.
//! 2. On successful run creation, the handler persists the prompt as
//!    the per-run `goal` default via `persist_run_string_default`
//!    (same mechanism the legacy orchestrate body's `goal` field uses
//!    — reading via `resolve_run_string_default(..., "goal")`).
//! 3. Added `#[serde(deny_unknown_fields)]` to `CreateRunRequest` so
//!    future typos (e.g. `prmopt`, `objective`) return 422 with the
//!    offending field name instead of being silently dropped.
//!
//! # Assertions
//!
//! 1. POST /v1/runs with a `prompt` returns 201.
//! 2. Orchestrating that run invokes the provider with a user message
//!    whose `## Goal` section is the exact prompt text — i.e. the
//!    operator's prompt REACHES THE LLM, not just storage.
//! 3. POST /v1/runs with an unknown field returns 422 naming the
//!    unknown field (product-quality UX: silent drops are a trap).
//! 4. POST /v1/runs with an explicit empty prompt returns 422 (an
//!    empty goal would reproduce the dogfood-v8 symptom silently).
//! 5. POST /v1/runs without `prompt` still works; a subsequent
//!    orchestrate call with `goal` in the body wins, proving the
//!    two code paths are not mutually exclusive.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

const MOCK_MODEL: &str = "openrouter/f42-prompt-threading";
const OPERATOR_PROMPT: &str =
    "Summarize the key design principles of Minecraft creative mode in three bullet points.";
const FINAL_ANSWER: &str = "Done.";

#[derive(Clone)]
struct MockState {
    hits: Arc<AtomicUsize>,
    /// Captured request bodies from every chat/completions call so
    /// the test can assert on the user message the LLM actually saw.
    bodies: Arc<Mutex<Vec<Value>>>,
}

async fn spawn_mock() -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<Value>>>) {
    let state = MockState {
        hits: Arc::new(AtomicUsize::new(0)),
        bodies: Arc::new(Mutex::new(Vec::new())),
    };
    let hits = state.hits.clone();
    let bodies = state.bodies.clone();

    async fn chat_handler(
        State(state): State<MockState>,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        state.hits.fetch_add(1, Ordering::SeqCst);
        state
            .bodies
            .lock()
            .expect("mock bodies mutex poisoned")
            .push(body);
        (
            StatusCode::OK,
            Json(json!({
                "id": "mock-f42",
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
                    "prompt_tokens": 40,
                    "completion_tokens": 12,
                    "total_tokens": 52,
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

    let base_url = format!("http://{addr}");
    let ready_url = format!("{base_url}/v1/models");
    let client = reqwest::Client::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Ok(r) = client.get(&ready_url).send().await {
            if r.status().is_success() {
                break;
            }
        }
        if std::time::Instant::now() >= deadline {
            panic!("mock provider at {ready_url} did not become ready within 2s");
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    (base_url, hits, bodies)
}

/// Provision tenant credential + provider connection + default model
/// so the orchestrator can route to the local mock. Factored out because
/// the four sub-tests here all need the same scaffold and inlining it
/// would make each test 100+ lines of noise.
async fn prepare_provider(h: &LiveHarness, tenant: &str, suffix: &str, mock_url: &str) {
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-f42-{suffix}"),
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
            "provider_connection_id": format!("conn_f42_{suffix}"),
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": [MOCK_MODEL],
            "credential_id": credential_id,
            "endpoint_url": mock_url,
        }))
        .send()
        .await
        .expect("connection reaches server");
    assert_eq!(r.status().as_u16(), 201);

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
        assert_eq!(
            r.status().as_u16(),
            200,
            "settings PUT for {key} must succeed"
        );
    }
}

async fn create_session(h: &LiveHarness, tenant: &str, workspace: &str, project: &str, session_id: &str) {
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
}

/// Primary F42 assertion: prompt on `POST /v1/runs` threads all the
/// way through to the LLM's user message `## Goal` section.
#[tokio::test]
async fn prompt_on_create_run_reaches_llm() {
    let h = LiveHarness::setup().await;
    let (mock_url, hits, bodies) = spawn_mock().await;

    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let session_id = format!("sess_f42_{suffix}");
    let run_id = format!("run_f42_{suffix}");

    prepare_provider(&h, &tenant, &suffix, &mock_url).await;
    create_session(&h, &tenant, &workspace, &project, &session_id).await;

    // Create the run with a prompt — this is the field that used to
    // be silently dropped.
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
            "prompt": OPERATOR_PROMPT,
        }))
        .send()
        .await
        .expect("run reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "POST /v1/runs with prompt must succeed: body={:?}",
        r.text().await.ok()
    );

    // Orchestrate WITHOUT a body.goal — the orchestrator must pick up
    // the prompt that was attached at run-creation time.
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "max_iterations": 4 }))
        .send()
        .await
        .expect("orchestrate reaches server");
    let status = r.status().as_u16();
    let body: Value = r.json().await.unwrap_or(Value::Null);
    assert_eq!(status, 200, "orchestrate must return 200; body={body}");
    let termination = body
        .get("termination")
        .and_then(|t| t.as_str())
        .unwrap_or("<missing>");
    assert_eq!(
        termination, "completed",
        "orchestrate must complete cleanly; body={body}"
    );

    // Exactly one provider hit (first turn emits complete_run).
    let n = hits.load(Ordering::SeqCst);
    assert_eq!(n, 1, "expected one provider hit; got {n}");

    // THE KEY ASSERTION: the mock provider received a user message
    // containing the operator's prompt text verbatim. This proves the
    // prompt travelled: handler → defaults store → resolve_run_string_default
    // → OrchestrationContext.goal → build_user_message → provider call.
    let captured = bodies.lock().expect("bodies mutex poisoned").clone();
    assert_eq!(captured.len(), 1, "expected exactly one captured body");
    let user_msg = captured[0]
        .get("messages")
        .and_then(|m| m.as_array())
        .expect("messages array present")
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .expect("user message present in chat request")
        .get("content")
        .and_then(|c| c.as_str())
        .expect("user content is string")
        .to_owned();

    assert!(
        user_msg.contains(OPERATOR_PROMPT),
        "F42: user message delivered to LLM must contain operator prompt \
         verbatim. Got:\n{user_msg}"
    );
    // Sanity: the old hardcoded placeholder must NOT appear — if it
    // did, the orchestrator fell back to the generic goal and the
    // prompt was silently dropped somewhere in the chain.
    assert!(
        !user_msg.contains("Execute the run objective."),
        "F42: user message must NOT fall back to the hardcoded placeholder. \
         Got:\n{user_msg}"
    );
}

/// F42 invariant: unknown fields on `CreateRunRequest` return 422 with
/// the offending field name. Silently dropping unknown fields was how
/// dogfood v8 hid the `prompt` drop for so long — `deny_unknown_fields`
/// converts that class of bug into a fail-fast error.
#[tokio::test]
async fn unknown_field_on_create_run_returns_422() {
    let h = LiveHarness::setup().await;

    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let session_id = format!("sess_f42b_{suffix}");
    let run_id = format!("run_f42b_{suffix}");

    create_session(&h, &tenant, &workspace, &project, &session_id).await;

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
            "frobnicate": "unexpected",
        }))
        .send()
        .await
        .expect("run reaches server");
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    assert_eq!(
        status, 422,
        "unknown field must return 422; got {status} body={body}"
    );
    assert!(
        body.contains("frobnicate"),
        "422 body must name the unknown field so operators can fix their client; \
         body={body}"
    );
}

/// F42 invariant: an explicit empty prompt is a 422, not a silent
/// fallback to the generic placeholder. Empty strings are almost
/// always a client bug (e.g. form field not bound); the old behaviour
/// would silently reproduce the dogfood-v8 symptom.
#[tokio::test]
async fn empty_prompt_on_create_run_returns_422() {
    let h = LiveHarness::setup().await;

    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let session_id = format!("sess_f42c_{suffix}");
    let run_id = format!("run_f42c_{suffix}");

    create_session(&h, &tenant, &workspace, &project, &session_id).await;

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
            "prompt": "   ",
        }))
        .send()
        .await
        .expect("run reaches server");
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    assert_eq!(
        status, 422,
        "empty/whitespace prompt must return 422; got {status} body={body}"
    );
    assert!(
        body.contains("prompt"),
        "422 body must name the offending field; body={body}"
    );
}

/// F42 back-compat: when `prompt` is omitted and a subsequent
/// `POST /v1/runs/:id/orchestrate` carries `goal`, that body-level
/// goal still wins. This documents the two code paths are additive,
/// not mutually exclusive — existing callers don't break.
#[tokio::test]
async fn orchestrate_body_goal_still_works_without_create_run_prompt() {
    let h = LiveHarness::setup().await;
    let (mock_url, _hits, bodies) = spawn_mock().await;

    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let session_id = format!("sess_f42d_{suffix}");
    let run_id = format!("run_f42d_{suffix}");

    prepare_provider(&h, &tenant, &suffix, &mock_url).await;
    create_session(&h, &tenant, &workspace, &project, &session_id).await;

    // No prompt on create.
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

    const LATE_GOAL: &str = "What is 2+2?";

    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "goal": LATE_GOAL, "max_iterations": 4 }))
        .send()
        .await
        .expect("orchestrate reaches server");
    assert_eq!(r.status().as_u16(), 200);

    let captured = bodies.lock().expect("bodies mutex poisoned").clone();
    let user_msg = captured[0]
        .get("messages")
        .and_then(|m| m.as_array())
        .unwrap()
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap()
        .to_owned();
    assert!(
        user_msg.contains(LATE_GOAL),
        "body.goal on /orchestrate must reach the LLM even when /runs \
         was created without a prompt. Got:\n{user_msg}"
    );
}
