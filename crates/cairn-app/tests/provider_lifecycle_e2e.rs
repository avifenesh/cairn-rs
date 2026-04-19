#![cfg(feature = "in-memory-runtime")]

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
    routing::post,
    Json, Router,
};
use cairn_api::{
    auth::{AuthPrincipal, ServiceTokenRegistry},
    bootstrap::BootstrapConfig,
};
use cairn_app::AppBootstrap;
use cairn_domain::tenancy::TenantKey;
#[allow(unused_imports)]
use cairn_domain::OperatorId;
use serde_json::{json, Value};
use tower::ServiceExt;

const TEST_TOKEN: &str = "provider-lifecycle-token";
const TEST_MODEL: &str = "openrouter/test-model";

fn register_token(tokens: &Arc<ServiceTokenRegistry>) {
    // Admin-gated credential-store route requires the admin service
    // account under the fail-closed `AdminRoleGuard` (T6b-C4).
    tokens.register(
        TEST_TOKEN.to_owned(),
        AuthPrincipal::ServiceAccount {
            name: "admin".to_owned(),
            tenant: TenantKey::new("default_tenant"),
        },
    );
}

fn clear_provider_env() {
    for key in [
        "CAIRN_BRAIN_URL",
        "CAIRN_BRAIN_MODEL",
        "CAIRN_WORKER_URL",
        "CAIRN_DEFAULT_GENERATE_MODEL",
        "CAIRN_DEFAULT_STREAM_MODEL",
        "CAIRN_DEFAULT_EMBED_MODEL",
        "OPENROUTER_API_KEY",
        "OLLAMA_HOST",
        "BEDROCK_API_KEY",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "AWS_REGION",
    ] {
        std::env::remove_var(key);
    }
}

async fn json_request(
    app: Router,
    method: Method,
    uri: &str,
    body: Value,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {TEST_TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn empty_request(app: Router, method: Method, uri: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {TEST_TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn spawn_openrouter_mock() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/chat/completions",
            post({
                let hits = hits.clone();
                move |Json(_body): Json<Value>| {
                    let hits = hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        Json(json!({
                            "id": "mock-openrouter",
                            "choices": [{
                                "index": 0,
                                "message": {
                                    "role": "assistant",
                                    "content": json!([{
                                        "action_type": "complete_run",
                                        "description": "dynamic provider finished the run",
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
                        }))
                    }
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post({
                let hits = hits.clone();
                move |Json(_body): Json<Value>| {
                    let hits = hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        Json(json!({
                            "id": "mock-openrouter-v1",
                            "choices": [{
                                "index": 0,
                                "message": {
                                    "role": "assistant",
                                    "content": json!([{
                                        "action_type": "complete_run",
                                        "description": "dynamic provider finished the run",
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
                        }))
                    }
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
async fn provider_lifecycle_routes_orchestration_through_connection_then_503_after_delete() {
    clear_provider_env();

    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    register_token(&tokens);

    let (mock_url, hits) = spawn_openrouter_mock().await;

    let credential_resp = json_request(
        app.clone(),
        Method::POST,
        "/v1/admin/tenants/default_tenant/credentials",
        json!({
            "provider_id": "openrouter",
            "plaintext_value": "sk-provider-lifecycle",
        }),
    )
    .await;
    assert_eq!(credential_resp.status(), StatusCode::CREATED);
    let credential_json = response_json(credential_resp).await;
    let credential_id = credential_json["id"]
        .as_str()
        .expect("credential id")
        .to_owned();

    let create_connection_resp = json_request(
        app.clone(),
        Method::POST,
        "/v1/providers/connections",
        json!({
            "tenant_id": "default_tenant",
            "provider_connection_id": "conn_provider_lifecycle",
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": [TEST_MODEL],
            "credential_id": credential_id,
            "endpoint_url": mock_url,
        }),
    )
    .await;
    assert_eq!(create_connection_resp.status(), StatusCode::CREATED);

    let registry_before =
        response_json(empty_request(app.clone(), Method::GET, "/v1/providers/registry").await)
            .await;
    assert_eq!(
        registry_before["connections"].as_array().unwrap().len(),
        0,
        "connections should stay uncached until first use"
    );
    assert!(
        registry_before["catalog"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == "openrouter"),
        "catalog payload should remain available for operator model pickers"
    );

    for key in ["generate_model", "brain_model"] {
        let response = json_request(
            app.clone(),
            Method::PUT,
            &format!("/v1/settings/defaults/system/system/{key}"),
            json!({ "value": TEST_MODEL }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "failed to set {key}");
    }

    let session_resp = json_request(
        app.clone(),
        Method::POST,
        "/v1/sessions",
        json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "sess_provider_lifecycle",
        }),
    )
    .await;
    assert_eq!(session_resp.status(), StatusCode::CREATED);

    let run_one_resp = json_request(
        app.clone(),
        Method::POST,
        "/v1/runs",
        json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "sess_provider_lifecycle",
            "run_id": "run_provider_lifecycle_1",
        }),
    )
    .await;
    assert_eq!(run_one_resp.status(), StatusCode::CREATED);

    let orchestrate_one = json_request(
        app.clone(),
        Method::POST,
        "/v1/runs/run_provider_lifecycle_1/orchestrate",
        json!({
            "goal": "Finish immediately",
            "max_iterations": 1,
        }),
    )
    .await;
    assert_eq!(orchestrate_one.status(), StatusCode::OK);
    let orchestrate_one_json = response_json(orchestrate_one).await;
    assert_eq!(orchestrate_one_json["termination"], "completed");
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let registry_after =
        response_json(empty_request(app.clone(), Method::GET, "/v1/providers/registry").await)
            .await;
    let connections = registry_after["connections"].as_array().unwrap();
    assert_eq!(connections.len(), 1);
    assert_eq!(connections[0]["connection_id"], "conn_provider_lifecycle");
    assert_eq!(connections[0]["backend"], "openrouter");
    assert_eq!(connections[0]["model"], TEST_MODEL);
    assert_eq!(connections[0]["cached"], true);
    assert_eq!(
        registry_after["fallbacks"].as_array().unwrap().len(),
        0,
        "sanitized test env should not expose startup fallbacks"
    );

    let delete_connection = empty_request(
        app.clone(),
        Method::DELETE,
        "/v1/providers/connections/conn_provider_lifecycle",
    )
    .await;
    assert_eq!(delete_connection.status(), StatusCode::OK);

    let registry_after_delete =
        response_json(empty_request(app.clone(), Method::GET, "/v1/providers/registry").await)
            .await;
    assert_eq!(
        registry_after_delete["connections"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "deleting the connection should invalidate the registry cache"
    );

    let run_two_resp = json_request(
        app.clone(),
        Method::POST,
        "/v1/runs",
        json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "sess_provider_lifecycle",
            "run_id": "run_provider_lifecycle_2",
        }),
    )
    .await;
    assert_eq!(run_two_resp.status(), StatusCode::CREATED);

    let orchestrate_two = json_request(
        app,
        Method::POST,
        "/v1/runs/run_provider_lifecycle_2/orchestrate",
        json!({
            "goal": "Try again without the dynamic connection",
            "max_iterations": 1,
        }),
    )
    .await;
    assert_eq!(orchestrate_two.status(), StatusCode::SERVICE_UNAVAILABLE);
    let orchestrate_two_json = response_json(orchestrate_two).await;
    assert_eq!(orchestrate_two_json["code"], "no_brain_provider");
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "second orchestration should not call the deleted provider"
    );
}
