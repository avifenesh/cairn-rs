//! Integration tests for RFC 014 entitlement enforcement.
//! Verifies that feature-gated endpoints return 403 in local_eval tier
//! and 201/200 in team_self_hosted tier.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use cairn_api::bootstrap::BootstrapConfig;
use cairn_app::AppBootstrap;
use cairn_domain::TenantId;
use tower::ServiceExt;

const TOKEN: &str = "entitlement-test-token";

async fn local_app() -> axum::Router {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(TOKEN, TenantId::new("default_tenant"));
    app
}

async fn team_app() -> axum::Router {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::team(
            "postgres://localhost/cairn_test",
        ))
        .await
        .unwrap();
    tokens.register(TOKEN, TenantId::new("default_tenant"));
    app
}

fn provider_connection_body() -> serde_json::Value {
    serde_json::json!({
        "tenant_id": "default_tenant",
        "provider_connection_id": "conn_test_1",
        "provider_family": "openai",
        "adapter_type": "responses_api"
    })
}

/// In local_eval tier, POST /v1/providers/connections must return 403.
#[tokio::test]
async fn entitlement_gates_provider_connection_denied_in_local_mode() {
    let app = local_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/providers/connections")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(provider_connection_body().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "local_eval tier should be denied multi_provider"
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["code"] == "entitlement_required",
        "response should carry entitlement_required code, got: {json}"
    );
}

/// In team_self_hosted tier, POST /v1/providers/connections must return 201.
#[tokio::test]
async fn entitlement_gates_provider_connection_allowed_in_team_mode() {
    let app = team_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/providers/connections")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(provider_connection_body().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "team_self_hosted tier should be allowed multi_provider"
    );
}
