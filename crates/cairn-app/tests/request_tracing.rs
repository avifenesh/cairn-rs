#![cfg(feature = "in-memory-runtime")]

//! Integration tests for RFC 011 request tracing and distributed trace IDs.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use cairn_api::auth::AuthPrincipal;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_app::AppBootstrap;
use cairn_domain::tenancy::TenantKey;
use cairn_domain::tenancy::WorkspaceRole;
use cairn_domain::{OperatorId, ProjectKey, SessionId, TenantId, WorkspaceId, WorkspaceKey};
use cairn_runtime::projects::ProjectService;
use cairn_runtime::{TenantService, WorkspaceMembershipService, WorkspaceService};
use tower::ServiceExt;

const TOKEN: &str = "tracing-test-token";
const TRACE_ID: &str = "my-trace-id-rfc011";

/// Set up: create tenant, workspace, project, session, and workspace membership.
async fn setup_project(app: &axum::Router, runtime: &cairn_runtime::InMemoryServices) {
    runtime
        .tenants
        .create(TenantId::new("trace_tenant"), "Trace Tenant".to_owned())
        .await
        .unwrap();

    runtime
        .workspaces
        .create(
            TenantId::new("trace_tenant"),
            WorkspaceId::new("trace_ws"),
            "Trace WS".to_owned(),
        )
        .await
        .unwrap();

    let ws_key = WorkspaceKey::new(TenantId::new("trace_tenant"), WorkspaceId::new("trace_ws"));
    // Add service_token as a member so create_run_handler passes role check.
    runtime
        .workspace_memberships
        .add_member(ws_key, "service_token".to_owned(), WorkspaceRole::Member)
        .await
        .unwrap();

    let project = ProjectKey::new("trace_tenant", "trace_ws", "trace_proj");
    runtime
        .projects
        .create(project.clone(), "Trace Project".to_owned())
        .await
        .unwrap();

    runtime
        .sessions
        .create(&project, SessionId::new("trace_sess"))
        .await
        .unwrap();

    let _ = app; // unused but shows dependency
}

#[tokio::test]
async fn request_tracing_run_creation_produces_spans() {
    let (app, runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("trace_tenant"),
        },
    );
    setup_project(&app, &runtime).await;

    // POST /v1/runs with a custom X-Trace-Id header.
    let body = serde_json::json!({
        "tenant_id": "trace_tenant",
        "workspace_id": "trace_ws",
        "project_id": "trace_proj",
        "session_id": "trace_sess",
        "run_id": "trace_run_1"
    });

    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .header("x-trace-id", TRACE_ID)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        create_resp.status(),
        StatusCode::CREATED,
        "run creation should succeed"
    );

    // Assert X-Trace-Id is in response headers.
    let resp_trace_id = create_resp
        .headers()
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        resp_trace_id, TRACE_ID,
        "X-Trace-Id response header should echo the incoming trace ID"
    );

    // GET /v1/trace/:trace_id
    let trace_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/trace/{TRACE_ID}"))
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(trace_resp.status(), StatusCode::OK);

    let body_bytes = to_bytes(trace_resp.into_body(), usize::MAX).await.unwrap();
    let trace_view: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(trace_view["trace_id"], TRACE_ID);

    let spans = trace_view["spans"].as_array().unwrap();
    assert!(
        !spans.is_empty(),
        "at least one span should be returned for the run creation, got: {trace_view}"
    );

    // At least one span should be for run_created.
    let has_run_created = spans
        .iter()
        .any(|s| s["event_type"].as_str() == Some("run_created"));
    assert!(
        has_run_created,
        "expected a run_created span, got: {spans:?}"
    );
}

#[tokio::test]
async fn request_tracing_x_trace_id_header_set_on_response() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
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
