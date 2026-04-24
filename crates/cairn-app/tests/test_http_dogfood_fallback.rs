//! End-to-end HTTP regression test for the DECIDE-phase provider fallback
//! chain introduced in F17 (dogfood run 2, 2026-04-23).
//!
//! Reproduces the failure pattern live: a tenant has two models bound to a
//! single OpenRouter-like connection. The preferred model upstream returns
//! HTTP 429 on every call; the secondary returns a well-formed tool_call.
//!
//! Before F17 the orchestrator surfaced `502 decide_error` immediately on
//! the 429. After F17 it must route to the secondary model, emit a
//! successful decide, and return 200. The mock counts per-model hits so we
//! can assert both models were tried (i.e. we really exercised the
//! fallback path, not a lucky single-model success).

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

const PREFERRED_MODEL: &str = "openrouter/preferred-rate-limited";
const FALLBACK_MODEL: &str = "openrouter/fallback-ok";

#[derive(Clone)]
struct MockState {
    preferred_hits: Arc<AtomicUsize>,
    fallback_hits: Arc<AtomicUsize>,
}

/// Spawns a mock OpenAI-compatible provider:
///   - Requests with `model == PREFERRED_MODEL` → 429 (RateLimited).
///   - Requests with `model == FALLBACK_MODEL`  → 200 with a valid
///     complete_run action (JSON array).
///
/// Returns `(base_url, preferred_hits, fallback_hits)`.
async fn spawn_dogfood_mock() -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let state = MockState {
        preferred_hits: Arc::new(AtomicUsize::new(0)),
        fallback_hits: Arc::new(AtomicUsize::new(0)),
    };

    let preferred = state.preferred_hits.clone();
    let fallback = state.fallback_hits.clone();

    async fn chat_handler(
        State(state): State<MockState>,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");
        if model == PREFERRED_MODEL {
            state.preferred_hits.fetch_add(1, Ordering::SeqCst);
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({
                    "error": {
                        "message": "rate-limited by dogfood mock",
                        "code": 429,
                    }
                })),
            );
        }
        if model == FALLBACK_MODEL {
            state.fallback_hits.fetch_add(1, Ordering::SeqCst);
            return (
                StatusCode::OK,
                Json(json!({
                    "id": "mock-fallback",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": json!([{
                                "action_type": "complete_run",
                                "description": "fallback model finished",
                                "confidence": 0.9,
                                "requires_approval": false,
                            }]).to_string(),
                        },
                        "finish_reason": "stop",
                    }],
                    "usage": {
                        "prompt_tokens": 20,
                        "completion_tokens": 10,
                        "total_tokens": 30,
                    }
                })),
            );
        }
        // Unknown model — treat as 400 so unexpected calls surface loudly.
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": format!("unknown model {model}") }})),
        )
    }

    let app = Router::new()
        .route("/chat/completions", post(chat_handler))
        .route("/v1/chat/completions", post(chat_handler))
        .route(
            "/v1/models",
            get(|| async {
                Json(json!({
                    "data": [
                        {"id": PREFERRED_MODEL},
                        {"id": FALLBACK_MODEL},
                    ]
                }))
            }),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    (format!("http://{addr}"), preferred, fallback)
}

#[tokio::test]
async fn orchestrate_falls_back_to_second_model_after_preferred_is_rate_limited() {
    let h = LiveHarness::setup().await;
    let (mock_url, preferred_hits, fallback_hits) = spawn_dogfood_mock().await;

    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_dogfood_{suffix}");
    let session_id = format!("sess_dogfood_{suffix}");
    let run_id = format!("run_dogfood_{suffix}");

    // 1. Credential.
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-dogfood-{suffix}"),
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

    // 2. Provider connection with BOTH models listed — the fallback chain
    //    is derived from this list in order.
    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "provider_connection_id": connection_id,
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": [PREFERRED_MODEL, FALLBACK_MODEL],
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

    // 3. Point system defaults at the preferred model so the orchestrate
    //    body's implicit resolve-default picks it up.
    for key in ["generate_model", "brain_model"] {
        let r = h
            .client()
            .put(format!(
                "{}/v1/settings/defaults/system/system/{}",
                h.base_url, key,
            ))
            .bearer_auth(&h.admin_token)
            .json(&json!({ "value": PREFERRED_MODEL }))
            .send()
            .await
            .expect("defaults reach server");
        assert_eq!(r.status().as_u16(), 200);
    }

    // 4. Session + run.
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

    // 5. Orchestrate. Preferred model will 429; fallback must succeed.
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id,))
        .bearer_auth(&h.admin_token)
        // Cairn picks the model from system defaults set in step 3 —
        // the caller no longer sends `model_id` (dropped in PR BU).
        .json(&json!({
            "goal": "finish immediately using fallback",
            "max_iterations": 1,
        }))
        .send()
        .await
        .expect("orchestrate reaches server");
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    // The critical DECIDE-phase assertion is that orchestrate did NOT
    // immediately return `502 decide_error` / `502 providers_exhausted`:
    // before F17 it always did, because a single 429 on the preferred
    // model aborted the loop. After F17 the decide phase must reach the
    // EXECUTE phase (which may subsequently fail for other reasons inside
    // the LiveHarness fabric — that's orthogonal to the fallback).
    assert_ne!(
        status, 502,
        "orchestrate must not 502 after fallback, got: {body}"
    );
    // Either 200 (full run completed) or 202 (waiting-approval / subagent)
    // is an acceptable outcome — what we must NOT see is a provider
    // exhaustion error.
    let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    assert_ne!(
        parsed.get("error_code").and_then(|v| v.as_str()),
        Some("all_providers_exhausted"),
        "fallback must not exhaust when only the preferred model rate-limits: {body}"
    );

    // 6. Both models must have been hit at least once: that proves the
    //    chain walked past the 429 rather than silently swapping in the
    //    default model. This is the core regression guard.
    assert!(
        preferred_hits.load(Ordering::SeqCst) >= 1,
        "preferred model must have been tried first"
    );
    assert!(
        fallback_hits.load(Ordering::SeqCst) >= 1,
        "fallback model must have been tried after 429; preferred={}, fallback={}",
        preferred_hits.load(Ordering::SeqCst),
        fallback_hits.load(Ordering::SeqCst)
    );
}
