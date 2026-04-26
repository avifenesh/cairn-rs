//! HTTP integration for tool-call approvals under the unified approval
//! surface (F45).
//!
//! Before F45 these endpoints lived at `/v1/tool-call-approvals/*`;
//! they now live under `/v1/approvals/*` with a `kind` discriminator in
//! responses. The pre-F45 paths are still live (`308 Permanent
//! Redirect`) and covered by `test_http_unified_approvals.rs`.
//!
//! Uses the `build_test_router_fake_fabric` harness — an InMemoryStore
//! wired router, no live Valkey — so these tests run as standard
//! `cargo test -p cairn-app` units.

mod support;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use cairn_api::auth::AuthPrincipal;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_domain::tenancy::TenantKey;
use cairn_domain::{
    ApprovalMatchPolicy, OperatorId, ProjectKey, RunId, SessionId, TenantId, ToolCallId,
};
use cairn_runtime::tool_call_approvals::ToolCallProposal;
use cairn_runtime::TenantService;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "tca-test-token";
const ACTOR: &str = "test_op";

fn bearer() -> String {
    format!("Bearer {TOKEN}")
}

fn seed_principal_for(state: &cairn_app::AppState, actor: &str, tenant: &str) {
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new(actor),
            tenant: TenantKey::new(tenant),
        },
    );
}

async fn seed_proposal(state: &cairn_app::AppState, call_id: &str) -> ToolCallProposal {
    let proposal = ToolCallProposal {
        call_id: ToolCallId::new(call_id),
        session_id: SessionId::new("sess_1"),
        run_id: RunId::new("run_1"),
        project: ProjectKey::new("default_tenant", "default_workspace", "default_project"),
        tool_name: "read".to_owned(),
        tool_args: json!({ "path": "/tmp/a.txt" }),
        display_summary: Some("read /tmp/a.txt".to_owned()),
        match_policy: ApprovalMatchPolicy::Exact,
    };
    state
        .runtime
        .tool_call_approvals
        .submit_proposal(proposal.clone())
        .await
        .expect("submit");
    proposal
}

async fn get_json(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", bearer());
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let body_bytes = match body {
        Some(v) => Body::from(serde_json::to_vec(&v).unwrap()),
        None => Body::empty(),
    };
    let res = app
        .oneshot(builder.body(body_bytes).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_returns_seeded_proposal() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal_for(&state, ACTOR, "default_tenant");
    seed_proposal(&state, "tc_list_1").await;

    let (status, body) = get_json(
        app,
        "GET",
        "/v1/approvals?kind=tool_call&run_id=run_1",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list: {body}");
    let items = body.get("items").and_then(Value::as_array).expect("items");
    let hit = items
        .iter()
        .find(|r| r["call_id"] == "tc_list_1")
        .expect("seeded proposal in list");
    assert_eq!(hit["kind"], "tool_call", "discriminator present");
}

#[tokio::test]
async fn get_returns_single_record() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal_for(&state, ACTOR, "default_tenant");
    seed_proposal(&state, "tc_get_1").await;

    let (status, body) = get_json(app, "GET", "/v1/approvals/tc_get_1", None).await;
    assert_eq!(status, StatusCode::OK, "get: {body}");
    assert_eq!(body["kind"], "tool_call");
    assert_eq!(body["call_id"], "tc_get_1");
    assert_eq!(body["state"], "pending");
}

#[tokio::test]
async fn get_returns_404_for_unknown_id() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal_for(&state, ACTOR, "default_tenant");

    let (status, _) = get_json(app, "GET", "/v1/approvals/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn approve_once_transitions_to_approved() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal_for(&state, ACTOR, "default_tenant");
    seed_proposal(&state, "tc_approve_1").await;

    let (status, body) = get_json(
        app,
        "POST",
        "/v1/approvals/tc_approve_1/approve",
        Some(json!({ "scope": { "type": "once" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "approve: {body}");
    assert_eq!(body["kind"], "tool_call");
    assert_eq!(body["state"], "approved");
    assert_eq!(body["operator_id"], ACTOR);
}

#[tokio::test]
async fn approve_session_falls_back_to_proposal_match_policy() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal_for(&state, ACTOR, "default_tenant");
    seed_proposal(&state, "tc_session_1").await;

    let (status, body) = get_json(
        app,
        "POST",
        "/v1/approvals/tc_session_1/approve",
        Some(json!({ "scope": { "type": "session" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "session approve: {body}");
    assert_eq!(body["state"], "approved");
    assert_eq!(body["scope"]["kind"], "session");
    assert_eq!(body["scope"]["match_policy"]["kind"], "exact");
}

#[tokio::test]
async fn reject_with_reason_transitions_to_rejected() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal_for(&state, ACTOR, "default_tenant");
    seed_proposal(&state, "tc_reject_1").await;

    let (status, body) = get_json(
        app,
        "POST",
        "/v1/approvals/tc_reject_1/reject",
        Some(json!({ "reason": "not this time" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "reject: {body}");
    assert_eq!(body["kind"], "tool_call");
    assert_eq!(body["state"], "rejected");
    assert_eq!(body["reason"], "not this time");
}

#[tokio::test]
async fn amend_updates_amended_args_without_resolving() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal_for(&state, ACTOR, "default_tenant");
    seed_proposal(&state, "tc_amend_1").await;

    let (status, body) = get_json(
        app,
        "PATCH",
        "/v1/approvals/tc_amend_1/amend",
        Some(json!({ "new_tool_args": { "path": "/tmp/b.txt" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "amend: {body}");
    assert_eq!(body["kind"], "tool_call");
    assert_eq!(body["state"], "pending");
    assert_eq!(body["amended_tool_args"]["path"], "/tmp/b.txt");
}

#[tokio::test]
async fn amend_self_targeting_is_forbidden() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal_for(&state, ACTOR, "default_tenant");
    let proposal = ToolCallProposal {
        call_id: ToolCallId::new("tc_self_amend"),
        session_id: SessionId::new("sess_1"),
        run_id: RunId::new("run_1"),
        project: ProjectKey::new("default_tenant", "default_workspace", "default_project"),
        tool_name: "amend_approval".to_owned(),
        tool_args: json!({}),
        display_summary: None,
        match_policy: ApprovalMatchPolicy::Exact,
    };
    state
        .runtime
        .tool_call_approvals
        .submit_proposal(proposal)
        .await
        .expect("submit");

    let (status, body) = get_json(
        app,
        "PATCH",
        "/v1/approvals/tc_self_amend/amend",
        Some(json!({ "new_tool_args": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "self-amend guard: {body}");
    assert_eq!(body["code"], "self_amend_forbidden");
}

#[tokio::test]
async fn operator_id_mismatch_is_rejected() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal_for(&state, ACTOR, "default_tenant");
    seed_proposal(&state, "tc_spoof").await;

    let (status, body) = get_json(
        app,
        "POST",
        "/v1/approvals/tc_spoof/approve",
        Some(json!({
            "operator_id": "not_me",
            "scope": { "type": "once" },
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "spoof: {body}");
    assert_eq!(body["code"], "identity_mismatch");
}

#[tokio::test]
async fn list_by_project_returns_pending_inbox() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal_for(&state, ACTOR, "default_tenant");
    seed_proposal(&state, "tc_inbox_1").await;
    seed_proposal(&state, "tc_inbox_2").await;
    state
        .runtime
        .tool_call_approvals
        .approve(
            ToolCallId::new("tc_inbox_2"),
            OperatorId::new(ACTOR),
            cairn_domain::ApprovalScope::Once,
            None,
        )
        .await
        .expect("approve");

    let (status, body) = get_json(
        app.clone(),
        "GET",
        "/v1/approvals?kind=tool_call&tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "inbox: {body}");
    let items = body["items"].as_array().unwrap();
    // Unified inbox only surfaces pending tool-call proposals via
    // `list_pending_for_project`. The resolved tc_inbox_2 doesn't show
    // up — operators who want resolved history scope by run/session.
    assert_eq!(items.len(), 1, "inbox returns only pending tool-calls");
    assert_eq!(items[0]["call_id"], "tc_inbox_1");
    assert_eq!(items[0]["state"], "pending");
}

#[tokio::test]
async fn cross_tenant_request_returns_404() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new(ACTOR),
            tenant: TenantKey::new("tenant_a"),
        },
    );
    state
        .runtime
        .tenants
        .create(TenantId::new("tenant_a"), "A".into())
        .await
        .ok();
    let proposal = ToolCallProposal {
        call_id: ToolCallId::new("tc_other_tenant"),
        session_id: SessionId::new("sess_o"),
        run_id: RunId::new("run_o"),
        project: ProjectKey::new("default_tenant", "default_workspace", "default_project"),
        tool_name: "read".to_owned(),
        tool_args: json!({ "path": "/tmp/a" }),
        display_summary: None,
        match_policy: ApprovalMatchPolicy::Exact,
    };
    state
        .runtime
        .tool_call_approvals
        .submit_proposal(proposal)
        .await
        .expect("submit");

    let (status, _) = get_json(app, "GET", "/v1/approvals/tc_other_tenant", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "cross-tenant must 404");
}
