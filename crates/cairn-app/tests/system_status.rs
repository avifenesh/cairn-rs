//! Integration tests for RFC 010 system status page.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use cairn_api::bootstrap::BootstrapConfig;
use cairn_app::AppBootstrap;
use cairn_domain::TenantId;
use tower::ServiceExt;

const TOKEN: &str = "system-status-token";

async fn app_with_token() -> (axum::Router, std::sync::Arc<cairn_runtime::InMemoryServices>) {
    let (app, runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(TOKEN, TenantId::new("default_tenant"));
    (app, runtime)
}

#[tokio::test]
async fn system_status_ok_when_all_healthy() {
    let (app, _runtime) = app_with_token().await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/status")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(status["status"], "ok", "all components healthy → overall ok, got: {status}");
    assert!(
        status["components"].as_array().unwrap().len() >= 3,
        "expected at least 3 components"
    );
    assert!(status["uptime_secs"].is_number());
    assert!(status["version"].is_string());
}

#[tokio::test]
async fn system_status_degraded_when_provider_marked_degraded() {
    use cairn_domain::{
        EventEnvelope, EventId, EventSource, ProviderConnectionId, ProviderMarkedDegraded,
        RuntimeEvent, TenantId as DomainTenantId,
    };
    use cairn_store::EventLog;

    let (app, runtime) = app_with_token().await;

    // Mark a provider as degraded by appending the event directly to the store.
    runtime
        .store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_prov_degrade_1"),
            EventSource::Runtime,
            RuntimeEvent::ProviderMarkedDegraded(ProviderMarkedDegraded {
                tenant_id: DomainTenantId::new("default_tenant"),
                connection_id: ProviderConnectionId::new("conn_degrade_1"),
                reason: "test degraded".to_owned(),
                marked_at_ms: 1_000,
            }),
        )])
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/status")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        status["status"], "degraded",
        "provider degraded → overall degraded, got: {status}"
    );

    let components = status["components"].as_array().unwrap();
    let provider_comp = components
        .iter()
        .find(|c| c["name"] == "provider_routing")
        .expect("provider_routing component missing");
    assert_eq!(provider_comp["status"], "degraded");
}
