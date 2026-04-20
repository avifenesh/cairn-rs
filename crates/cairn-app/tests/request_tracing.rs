//! Integration tests for RFC 011 request tracing and distributed trace IDs.
//!
//! The run-creation-produces-spans test (formerly here, which drove
//! `POST /v1/runs` through `runs.start_command`) migrated to
//! `crates/cairn-fabric/tests/integration/test_request_tracing.rs` in
//! the kill-in-memory-runtime work — the span assertion is only
//! meaningful when the full run-create path runs end-to-end, and that
//! path now lives on Fabric.

mod support;

use axum::{body::Body, http::Request};
use cairn_api::auth::AuthPrincipal;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_domain::tenancy::TenantKey;
use cairn_domain::OperatorId;
use tower::ServiceExt;

const TOKEN: &str = "tracing-test-token";

#[tokio::test]
async fn request_tracing_x_trace_id_header_set_on_response() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    // Any request should get X-Trace-Id back.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.headers().contains_key("x-trace-id"),
        "every response must carry x-trace-id"
    );
    assert!(
        resp.headers().contains_key("x-request-id"),
        "every response must carry x-request-id"
    );
}
