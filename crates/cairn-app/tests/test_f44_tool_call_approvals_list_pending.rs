//! F44 regression (carried onto the F45 unified surface): the
//! `GET /v1/approvals` list endpoint must surface pending tool-call
//! approvals that are already visible via `GET /v1/approvals/:id`.
//!
//! Dogfood M1 (2026-04-26) hit: orchestrate created a pending tool-call
//! approval whose by-id fetch returned a record in `state=pending`,
//! but the list endpoint returned `{items:[], hasMore:false}`. The
//! operator inbox in the UI consumes the list endpoint and was blank
//! while the approval was live. The bug report used `?status=pending`;
//! `status` is not a supported query key (the server accepts `state`)
//! and was silently dropped by the handler, so the response was the
//! full unfiltered list on an admin token — it looked fine and was
//! actually wrong.
//!
//! F45 unified these paths under `/v1/approvals/*` with a `kind`
//! discriminator; the legacy `/v1/tool-call-approvals/*` paths now
//! 308-redirect and are covered in `test_http_unified_approvals.rs`.
//! This file asserts the same F44 regressions against the new surface
//! to prove unification did not re-open the bug.
//!
//! This test suite pins five properties:
//!
//!   1. A pending approval produced by `ToolCallApprovalService::submit_proposal`
//!      is listed by `?run_id=...&state=pending`.
//!   2. The same pending record is listed by the project-triple inbox
//!      query (admin caller, explicit tenant/workspace/project params).
//!   3. An admin caller with NO scope params sees pending approvals
//!      from the canonical `default_tenant` project (the super-admin
//!      inbox path — previously fell back to the admin service
//!      account's own tenant and returned `[]`).
//!   4. An unsupported query key such as `status=pending` is rejected
//!      loudly with 400 rather than silently dropped — the silent-drop
//!      behaviour is what made the bug invisible during the dogfood
//!      run.
//!   5. After approve, the record drops out of the pending list and
//!      appears in `?state=approved` for the run scope.

mod support;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use cairn_api::auth::AuthPrincipal;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_domain::tenancy::TenantKey;
use cairn_domain::{
    ApprovalMatchPolicy, ApprovalScope, OperatorId, ProjectKey, RunId, SessionId, TenantId,
    ToolCallId,
};
use cairn_runtime::tool_call_approvals::ToolCallProposal;
use cairn_runtime::TenantService;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "f44-test-token";
const ACTOR: &str = "f44_operator";

fn bearer() -> String {
    format!("Bearer {TOKEN}")
}

/// Register a service-account principal that is flagged as `admin` by
/// `is_admin_principal` (name == "admin"). This mirrors the real admin
/// token wiring in `main.rs`, which is the surface that tripped F44:
/// admin tokens bypass tenant filters, so a stray empty-set filter is
/// the only reason the list could return nothing while by-id returns
/// the record.
fn seed_admin_principal(state: &cairn_app::AppState, tenant: &str) {
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::ServiceAccount {
            name: "admin".to_owned(),
            tenant: TenantKey::new(tenant),
        },
    );
}

async fn seed_pending_proposal(
    state: &cairn_app::AppState,
    call_id: &str,
    run_id: &str,
) -> ToolCallProposal {
    let proposal = ToolCallProposal {
        call_id: ToolCallId::new(call_id),
        session_id: SessionId::new("sess_f44"),
        run_id: RunId::new(run_id),
        project: ProjectKey::new("default_tenant", "default_workspace", "default_project"),
        tool_name: "read".to_owned(),
        tool_args: json!({ "path": "/tmp/dogfood.txt" }),
        display_summary: Some("read /tmp/dogfood.txt".to_owned()),
        match_policy: ApprovalMatchPolicy::Exact,
    };
    state
        .runtime
        .tool_call_approvals
        .submit_proposal(proposal.clone())
        .await
        .expect("submit_proposal must persist the ToolCallProposed event");
    proposal
}

async fn http(
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
        .expect("router must respond");
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, parsed)
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// Core regression: when orchestrate proposes a tool call, the operator
/// inbox list (`state=pending`, scoped by run) must include it. The by-id
/// path already returns the record — any discrepancy between the two
/// views is the exact F44 symptom.
#[tokio::test]
async fn f44_pending_approval_is_listed_by_run_id() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_admin_principal(&state, "default");
    let proposal = seed_pending_proposal(&state, "tc_f44_run", "run_f44").await;

    // 1. Sanity — by-id works, state=pending.
    let (status, body) = http(
        app.clone(),
        "GET",
        "/v1/approvals/tc_f44_run",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "by-id must return 200: {body}");
    assert_eq!(body["call_id"], proposal.call_id.as_str());
    assert_eq!(body["state"], "pending");

    // 2. The F44 bug path: list by run_id + state=pending must return
    //    the same record.
    let (status, body) = http(
        app,
        "GET",
        "/v1/approvals?run_id=run_f44&state=pending",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list must return 200: {body}");
    let items = body
        .get("items")
        .and_then(Value::as_array)
        .expect("list response must carry items array");
    assert_eq!(
        items.len(),
        1,
        "pending approval present via by-id must also be listed: {body}"
    );
    assert_eq!(items[0]["call_id"], proposal.call_id.as_str());
    assert_eq!(items[0]["state"], "pending");
    assert_eq!(body["hasMore"], false);
}

/// Project-triple inbox path (the UI's default ApprovalsPage query): no
/// run_id / session_id, admin caller, scope-params match the proposal's
/// project. Must surface the pending record.
#[tokio::test]
async fn f44_pending_approval_is_listed_by_project_inbox_as_admin() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_admin_principal(&state, "default");
    let proposal = seed_pending_proposal(&state, "tc_f44_inbox", "run_f44_inbox").await;

    let (status, body) = http(
        app,
        "GET",
        "/v1/approvals?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "inbox must return 200: {body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        items
            .iter()
            .any(|r| r["call_id"] == proposal.call_id.as_str()),
        "project inbox must include the pending proposal: {body}"
    );
}

/// Admin caller with NO scope params — this is the UI shape when the
/// operator has not drilled into a tenant yet. Before F44 the handler
/// fell back to `tenant_scope.tenant_id()`, which for the admin token
/// is the admin service-account's tenant (`default` — NOT
/// `default_tenant` where DEFAULT_SCOPE lives), so the inbox query hit
/// an empty cell and returned `[]` while by-id served the record.
#[tokio::test]
async fn f44_admin_inbox_without_scope_params_defaults_to_canonical_project() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    // Admin registered under tenant "default" — mirrors main.rs line 568.
    seed_admin_principal(&state, "default");
    let proposal = seed_pending_proposal(&state, "tc_f44_noscope", "run_f44_noscope").await;

    let (status, body) = http(app, "GET", "/v1/approvals", None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "admin inbox without scope params must return 200: {body}"
    );
    let items = body["items"].as_array().expect("items");
    assert!(
        items
            .iter()
            .any(|r| r["call_id"] == proposal.call_id.as_str()),
        "admin inbox without explicit scope must surface default_tenant \
         approvals (the canonical cell where orchestrate + UI write): {body}"
    );
}

/// `status=pending` is the literal param name dogfood users tried in
/// the bug report. It is NOT a supported query key — the handler
/// accepts `state=...`. Before F44 an unknown key was silently ignored
/// so the list appeared empty despite a live pending record. After F44
/// the handler rejects unknown filter keys loudly (400 via
/// `deny_unknown_fields`) so operators see the typo rather than a
/// misleading empty list.
#[tokio::test]
async fn f44_unknown_filter_key_is_rejected_loudly() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_admin_principal(&state, "default");
    seed_pending_proposal(&state, "tc_f44_typo", "run_f44_typo").await;

    let (status, body) = http(
        app,
        "GET",
        "/v1/approvals?status=pending&run_id=run_f44_typo",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "`status` (alias of `state`) must be rejected rather than silently returning \
         the full unfiltered list: {body}"
    );
}

/// State-transition sanity: after approve, the record leaves the pending
/// list and appears in `state=approved` for the run scope.
#[tokio::test]
async fn f44_approve_moves_record_between_state_buckets() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_admin_principal(&state, "default");
    // Also seed a regular tenant so the domain service accepts writes
    // if it ever enforces tenant existence.
    let _ = state
        .runtime
        .tenants
        .create(TenantId::new("default_tenant"), "default".into())
        .await;
    seed_pending_proposal(&state, "tc_f44_flow", "run_f44_flow").await;

    // Before approve: pending list contains the call, approved list is empty.
    let (_, body) = http(
        app.clone(),
        "GET",
        "/v1/approvals?run_id=run_f44_flow&state=pending",
        None,
    )
    .await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    let (_, body) = http(
        app.clone(),
        "GET",
        "/v1/approvals?run_id=run_f44_flow&state=approved",
        None,
    )
    .await;
    assert!(body["items"].as_array().unwrap().is_empty());

    // Approve out-of-band via the service (HTTP approve path has its own
    // coverage in test_http_tool_call_approvals; here we only care about
    // the list projection).
    state
        .runtime
        .tool_call_approvals
        .approve(
            ToolCallId::new("tc_f44_flow"),
            OperatorId::new(ACTOR),
            ApprovalScope::Once,
            None,
        )
        .await
        .expect("approve");

    // After approve: pending list empty, approved list contains the call.
    let (_, body) = http(
        app.clone(),
        "GET",
        "/v1/approvals?run_id=run_f44_flow&state=pending",
        None,
    )
    .await;
    assert!(
        body["items"].as_array().unwrap().is_empty(),
        "pending list must NOT include approved record: {body}"
    );

    let (status, body) = http(
        app,
        "GET",
        "/v1/approvals?run_id=run_f44_flow&state=approved",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        1,
        "approved list must contain the record: {body}"
    );
    assert_eq!(items[0]["call_id"], "tc_f44_flow");
    assert_eq!(items[0]["state"], "approved");
}
