//! Integration test for RFC 008 tenant overview endpoint.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use cairn_api::bootstrap::BootstrapConfig;
use cairn_app::AppBootstrap;
use cairn_domain::{TenantId, WorkspaceId, WorkspaceKey};
use cairn_domain::tenancy::WorkspaceRole;
use cairn_runtime::{TenantService, WorkspaceMembershipService, WorkspaceService};
use tower::ServiceExt;

const TOKEN: &str = "tenant-overview-token";

#[tokio::test]
async fn tenant_overview_returns_workspace_and_member_counts() {
    let (app, runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(TOKEN, TenantId::new("default_tenant"));

    // 1. Create tenant
    runtime
        .tenants
        .create(TenantId::new("acme"), "Acme Corp".to_owned())
        .await
        .unwrap();

    // 2. Create workspace_a and workspace_b
    runtime
        .workspaces
        .create(
            TenantId::new("acme"),
            WorkspaceId::new("ws_a"),
            "Workspace A".to_owned(),
        )
        .await
        .unwrap();

    runtime
        .workspaces
        .create(
            TenantId::new("acme"),
            WorkspaceId::new("ws_b"),
            "Workspace B".to_owned(),
        )
        .await
        .unwrap();

    let ws_a_key = WorkspaceKey::new(TenantId::new("acme"), WorkspaceId::new("ws_a"));
    let ws_b_key = WorkspaceKey::new(TenantId::new("acme"), WorkspaceId::new("ws_b"));

    // 3. Add 3 members to ws_a
    runtime
        .workspace_memberships
        .add_member(ws_a_key.clone(), "alice".to_owned(), WorkspaceRole::Admin)
        .await
        .unwrap();
    runtime
        .workspace_memberships
        .add_member(ws_a_key.clone(), "bob".to_owned(), WorkspaceRole::Member)
        .await
        .unwrap();
    runtime
        .workspace_memberships
        .add_member(ws_a_key.clone(), "carol".to_owned(), WorkspaceRole::Viewer)
        .await
        .unwrap();

    // 4. Add 1 member to ws_b
    runtime
        .workspace_memberships
        .add_member(ws_b_key.clone(), "dave".to_owned(), WorkspaceRole::Member)
        .await
        .unwrap();

    // 5. GET /v1/admin/tenants/acme/overview
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/admin/tenants/acme/overview")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let overview: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Assert top-level counts
    assert_eq!(
        overview["workspace_count"], 2,
        "expected 2 workspaces, got: {overview}"
    );
    assert_eq!(
        overview["total_members"], 4,
        "expected 4 total members, got: {overview}"
    );

    // Find ws_a summary and assert member_count=3
    let workspaces = overview["workspaces"].as_array().unwrap();
    let ws_a_summary = workspaces
        .iter()
        .find(|w| w["workspace_id"] == "ws_a")
        .expect("ws_a not found in workspaces");

    assert_eq!(
        ws_a_summary["member_count"], 3,
        "ws_a should have 3 members, got: {ws_a_summary}"
    );
    assert_eq!(ws_a_summary["name"], "Workspace A");

    let ws_b_summary = workspaces
        .iter()
        .find(|w| w["workspace_id"] == "ws_b")
        .expect("ws_b not found in workspaces");

    assert_eq!(
        ws_b_summary["member_count"], 1,
        "ws_b should have 1 member, got: {ws_b_summary}"
    );
}

#[tokio::test]
async fn tenant_overview_returns_404_for_unknown_tenant() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(TOKEN, TenantId::new("default_tenant"));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/admin/tenants/no_such_tenant/overview")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
