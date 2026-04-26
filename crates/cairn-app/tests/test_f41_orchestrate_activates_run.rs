//! F41 regression: `POST /v1/runs/:id/orchestrate` must activate the
//! run's FF execution before the loop dispatches any terminal FCALL.
//!
//! # Context
//!
//! Dogfood v6 (2026-04-26, CG-b binary, Z.ai glm-4.7) produced three
//! terminal failures for three trivial prompts:
//!
//! ```text
//! {"reason":"invalid run transition: execution_not_active -> completed",
//!  "termination":"failed"}
//! ```
//!
//! Task-2 failed at iteration 0 — before any provider call. That ruled
//! out mid-loop lease expiry and pointed at "never activated": FF's
//! terminal FCALLs (`ff_complete_execution`, `ff_fail_execution`,
//! `ff_cancel_execution`) gate on `lifecycle_phase == "active"` via
//! `validate_lease_and_mark_expired`, and `ff_create_execution` (which
//! backs `runs.start`) leaves the execution in `lifecycle_phase =
//! "runnable"`. The only transition to active is `ff_claim_execution`
//! via `issue_grant_and_claim` — previously reachable only via
//! `POST /v1/runs/:id/claim`, which the orchestrate flow did not call.
//!
//! # Fix
//!
//! [`FabricRunService::ensure_active`] is an idempotent on-ramp:
//! snapshot-check → short-circuit if `current_lease.is_some()` →
//! otherwise walk `issue_grant_and_claim`. The orchestrate handler
//! calls it right after the Pending → Running transition so the
//! terminal FCALL path can succeed regardless of whether the caller
//! explicitly claimed.
//!
//! # This test
//!
//! Stands up a mock OpenAI-compatible provider that emits a
//! native `complete_run` tool_call on its first turn, then asserts:
//!
//!   1. `POST /v1/runs/:id/orchestrate` returns 200 with
//!      `termination == "completed"` (NOT `"failed"` with the
//!      `execution_not_active` reason).
//!   2. The run's `summary` field carries the `final_answer` argument
//!      from the tool_call — i.e. the terminal FCALL actually landed
//!      in FF, not just in cairn's projection.
//!
//! Without the F41 fix, the LiveHarness path would hit FF's typed
//! rejection and `termination` would be `"failed"` with the raw
//! classifier message — reproducing dogfood v6 one-for-one.

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

const MOCK_MODEL: &str = "openrouter/f41-ensure-active";
const FINAL_ANSWER: &str = "The capital of France is Paris.";

#[derive(Clone)]
struct MockState {
    hits: Arc<AtomicUsize>,
}

async fn spawn_mock() -> (String, Arc<AtomicUsize>) {
    let state = MockState {
        hits: Arc::new(AtomicUsize::new(0)),
    };
    let hits = state.hits.clone();

    async fn chat_handler(
        State(state): State<MockState>,
        Json(_body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        state.hits.fetch_add(1, Ordering::SeqCst);
        (
            StatusCode::OK,
            Json(json!({
                "id": "mock-f41",
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

    // Deterministic readiness probe — a fixed sleep would flake on a
    // loaded CI runner. Poll /v1/models for up to 2s with a 20ms
    // backoff; each iteration is one TCP attempt so the happy path
    // returns after <5ms.
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
    (base_url, hits)
}

#[tokio::test]
async fn orchestrate_completes_without_manual_claim() {
    let h = LiveHarness::setup().await;
    let (mock_url, hits) = spawn_mock().await;

    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_f41_{suffix}");
    let session_id = format!("sess_f41_{suffix}");
    let run_id = format!("run_f41_{suffix}");

    // Provider credential + connection.
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-f41-{suffix}"),
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
        assert_eq!(r.status().as_u16(), 200);
    }

    // Session + run. IMPORTANTLY: do NOT POST /v1/runs/:id/claim.
    // This mirrors the dogfood v6 call sequence that dashboard +
    // external callers make; without F41 the terminal FCALL on this
    // path would reject with execution_not_active.
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

    // Orchestrate.
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id,))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "What is the capital of France?",
            "max_iterations": 4,
        }))
        .send()
        .await
        .expect("orchestrate reaches server");
    let status = r.status().as_u16();
    let body: Value = r.json().await.unwrap_or(Value::Null);

    assert_eq!(status, 200, "orchestrate must return 200; body={body}");

    // Exactly one provider hit (native complete_run on turn 1).
    let n = hits.load(Ordering::SeqCst);
    assert_eq!(n, 1, "expected one provider hit; got {n}. body={body}");

    // F41 primary assertion: termination is "completed", not "failed".
    //
    // Pre-fix this was `{"termination":"failed",
    // "reason":"invalid run transition: execution_not_active -> completed"}`.
    // The F37 classifier still works as intended — we just stopped
    // putting the run in a state that trips it.
    let termination = body
        .get("termination")
        .and_then(|t| t.as_str())
        .unwrap_or("<missing>");
    assert_eq!(
        termination, "completed",
        "F41: orchestrate must complete cleanly without a manual /claim step; \
         body={body}"
    );

    // The terminal FCALL landed in FF AND cairn's `complete` call
    // succeeded — so the summary returned to the HTTP caller carries
    // the tool_call's `final_answer` verbatim.
    assert_eq!(
        body.get("summary").and_then(|s| s.as_str()),
        Some(FINAL_ANSWER),
        "F41: terminal FCALL must have accepted and summary must \
         propagate; body={body}",
    );
}

/// F41 secondary: calling `/claim` explicitly before `/orchestrate`
/// must still work (and must not be double-punished by FF's
/// `grant_already_exists` contention rejection). `ensure_active`'s
/// idempotency guard is load-bearing for this path.
#[tokio::test]
async fn orchestrate_after_explicit_claim_is_idempotent() {
    let h = LiveHarness::setup().await;
    let (mock_url, _hits) = spawn_mock().await;

    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_f41b_{suffix}");
    let session_id = format!("sess_f41b_{suffix}");
    let run_id = format!("run_f41b_{suffix}");

    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-f41b-{suffix}"),
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
        // Assert the status so a future setting-endpoint regression
        // surfaces here rather than as a confusing
        // no_brain_provider / no_generate_provider downstream error.
        assert_eq!(
            r.status().as_u16(),
            200,
            "settings PUT for {key} must succeed"
        );
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

    // Explicit claim first — simulates dashboard paths that claim
    // before orchestrating.
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/claim", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("claim reaches server");
    assert_eq!(
        r.status().as_u16(),
        200,
        "claim must succeed; body={:?}",
        r.text().await.ok()
    );

    // Now orchestrate. Without the idempotency guard, `ensure_active`
    // would trip FF's `grant_already_exists` and surface as a 409 /
    // 500 here. With the guard, the snapshot check short-circuits.
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id,))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "What is 2+2?",
            "max_iterations": 4,
        }))
        .send()
        .await
        .expect("orchestrate reaches server");
    let status = r.status().as_u16();
    let body: Value = r.json().await.unwrap_or(Value::Null);

    assert_eq!(
        status, 200,
        "orchestrate after explicit claim must still succeed (ensure_active \
         must be idempotent); status={status} body={body}"
    );
    let termination = body
        .get("termination")
        .and_then(|t| t.as_str())
        .unwrap_or("<missing>");
    assert_eq!(
        termination, "completed",
        "orchestrate after explicit claim must complete; body={body}"
    );
}
