//! Provider lifecycle e2e: credential -> connection -> defaults ->
//! orchestrate routes through connection -> delete -> orchestrate 503.
//!
//! Migrated from the feature-gated `provider_lifecycle_e2e.rs` removed
//! in PR #67. That test couldn't run under the live Fabric runtime
//! (the in-memory-runtime feature carried the whole fixture). This is
//! its replacement against a real cairn-app subprocess.
//!
//! Covers task #121.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

const TEST_MODEL: &str = "openrouter/test-model";

async fn spawn_openrouter_mock() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let chat_response = || {
        json!({
            "id": "mock-openrouter",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": json!([{
                        "action_type": "complete_run",
                        "description": "mock finished the run",
                        "confidence": 0.99,
                    }]).to_string(),
                },
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens": 11,
                "completion_tokens": 7,
                "total_tokens": 18,
            }
        })
    };

    let hits_a = hits.clone();
    let hits_b = hits.clone();
    let hits_c = hits.clone();
    let app = Router::new()
        .route(
            "/chat/completions",
            post(move |Json(_): Json<Value>| {
                let hits = hits_a.clone();
                let resp = chat_response();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Json(resp)
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move |Json(_): Json<Value>| {
                let hits = hits_b.clone();
                let resp = chat_response();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Json(resp)
                }
            }),
        )
        .route(
            "/v1/models",
            get(move || {
                let hits = hits_c.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"data": [{"id": TEST_MODEL}]}))
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    (format!("http://{addr}"), hits)
}

#[tokio::test]
async fn provider_connection_routes_orchestration_then_503_after_delete() {
    let h = LiveHarness::setup().await;
    let (mock_url, hits) = spawn_openrouter_mock().await;

    // Uuid-scope inside the "default_tenant" admin space so parallel
    // runs of this test in the same cargo process don't collide on
    // credential_id / connection_id — both are globally unique per
    // subprocess tenant.
    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_{suffix}");
    let session_id = format!("sess_{suffix}");
    let run_id_1 = format!("run1_{suffix}");
    let run_id_2 = format!("run2_{suffix}");

    // 1. Create credential in the tenant's credential store.
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-test-{suffix}"),
        }))
        .send()
        .await
        .expect("credential create reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "credential: {}",
        r.text().await.unwrap_or_default(),
    );
    let credential_id = r
        .json::<Value>()
        .await
        .expect("credential json")
        .get("id")
        .and_then(|v| v.as_str())
        .expect("credential id")
        .to_owned();

    // 2. Create provider connection wired to the mock endpoint.
    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "provider_connection_id": connection_id,
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": [TEST_MODEL],
            "credential_id": credential_id,
            "endpoint_url": mock_url,
        }))
        .send()
        .await
        .expect("connection create reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "connection: {}",
        r.text().await.unwrap_or_default(),
    );

    // 3. Point system defaults at the test model so orchestrate picks
    //    our connection.
    for key in ["generate_model", "brain_model"] {
        let r = h
            .client()
            .put(format!(
                "{}/v1/settings/defaults/system/system/{}",
                h.base_url, key,
            ))
            .bearer_auth(&h.admin_token)
            .json(&json!({ "value": TEST_MODEL }))
            .send()
            .await
            .expect("settings put reaches server");
        assert_eq!(r.status().as_u16(), 200, "settings {key}");
    }

    // 4. Session + run 1.
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
            "run_id": run_id_1,
        }))
        .send()
        .await
        .expect("run1 reaches server");
    assert_eq!(r.status().as_u16(), 201);

    // 5. Orchestrate run 1 — must route through the mock.
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id_1,))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "finish immediately",
            "max_iterations": 1,
        }))
        .send()
        .await
        .expect("orchestrate 1 reaches server");
    assert_eq!(
        r.status().as_u16(),
        200,
        "orchestrate 1: {}",
        r.text().await.unwrap_or_default(),
    );
    assert!(
        hits.load(Ordering::SeqCst) >= 1,
        "mock must have been hit at least once, hits={}",
        hits.load(Ordering::SeqCst),
    );

    // 6. Delete the connection. Registry cache must invalidate.
    let r = h
        .client()
        .delete(format!(
            "{}/v1/providers/connections/{}",
            h.base_url, connection_id,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("delete reaches server");
    assert_eq!(
        r.status().as_u16(),
        200,
        "delete: {}",
        r.text().await.unwrap_or_default(),
    );

    // 7. Run 2 + orchestrate: must now 503 because no backing connection.
    let r = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "session_id": session_id,
            "run_id": run_id_2,
        }))
        .send()
        .await
        .expect("run2 reaches server");
    assert_eq!(r.status().as_u16(), 201);

    let hits_before = hits.load(Ordering::SeqCst);
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id_2,))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "try again",
            "max_iterations": 1,
        }))
        .send()
        .await
        .expect("orchestrate 2 reaches server");
    // Post-delete orchestration must NOT succeed. The exact upstream-
    // unavailable status depends on which layer short-circuits first
    // (provider registry -> 503 no_brain_provider, orchestrator decide
    // phase -> 502 decide_error); either is a valid operator-facing
    // signal. The critical invariant is: no 2xx, no 5xx-internal, and
    // the mock never gets hit through a stale connection.
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    assert!(
        matches!(status, 502 | 503),
        "orchestrate 2 post-delete expected 502 or 503, got {status}: {body}",
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        hits_before,
        "mock must not be hit after connection delete",
    );
}

/// Regression for issue #156: when a tenant has an active connection but the
/// caller asks for a model the connection doesn't serve, chat/stream must
/// return 422 with an actionable error — not the old 503 "no provider
/// configured" that pointed the operator at the wrong env vars.
#[tokio::test]
async fn chat_stream_returns_422_when_model_not_supported_by_any_connection() {
    let h = LiveHarness::setup().await;
    let (mock_url, _hits) = spawn_openrouter_mock().await;

    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let connection_id = format!("conn422_{suffix}");

    // Register a credential + connection whose supported_models is empty —
    // exactly the shape ProvidersPage produces if the operator skips the
    // models step.
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-test-422-{suffix}"),
        }))
        .send()
        .await
        .expect("credential reaches server");
    assert_eq!(r.status().as_u16(), 201);
    let credential_id = r
        .json::<Value>()
        .await
        .unwrap()
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
            "supported_models": [],
            "credential_id": credential_id,
            "endpoint_url": mock_url,
        }))
        .send()
        .await
        .expect("connection reaches server");
    assert_eq!(r.status().as_u16(), 201);

    // Chat stream with a model no connection serves.
    let r = h
        .client()
        .post(format!("{}/v1/chat/stream", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "model": "openrouter/definitely-not-registered",
            "prompt": "hello",
        }))
        .send()
        .await
        .expect("chat/stream reaches server");

    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    assert_eq!(
        status, 422,
        "expected 422 when no active connection supports the model, got {status}: {body}",
    );
    assert!(
        body.contains("No registered connection"),
        "422 body must explain the root cause, got: {body}",
    );
    assert!(
        body.contains("discover-models") || body.contains("supported_models"),
        "422 body must point at the fix, got: {body}",
    );
    assert!(
        body.contains("active_connections"),
        "422 body must enumerate active connections, got: {body}",
    );
}
