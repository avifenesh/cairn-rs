//! Unified approval surface integration tests (F45).
//!
//! Covers:
//! 1. End-to-end tool-call approval round trip via `/v1/approvals`:
//!    propose → list → approve.
//! 2. End-to-end plan approval round trip via `/v1/approvals`:
//!    request → list → approve.
//! 3. Merged list returns both kinds with a `kind` discriminator.
//! 4. `/v1/tool-call-approvals/*` responds with 308 redirect to the
//!    unified path, and the redirect target works.

mod support;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use cairn_api::auth::AuthPrincipal;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_domain::tenancy::TenantKey;
use cairn_domain::{
    ApprovalId, ApprovalMatchPolicy, ApprovalRequirement, OperatorId, ProjectKey, RunId, SessionId,
    ToolCallId,
};
use cairn_runtime::tool_call_approvals::ToolCallProposal;
use cairn_runtime::ApprovalService;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "unified-approvals-token";
const ACTOR: &str = "test_op";

fn bearer() -> String {
    format!("Bearer {TOKEN}")
}

fn register_principal(state: &cairn_app::AppState) {
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new(ACTOR),
            tenant: TenantKey::new("default_tenant"),
        },
    );
}

async fn call(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
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
    let headers = res.headers().clone();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let json_body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json_body, headers)
}

// ── 1. Tool-call round trip via unified surface ────────────────────────────

#[tokio::test]
async fn tool_call_round_trip_via_unified_surface() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    register_principal(&state);

    // Seed a proposal the way orchestrate does internally.
    let proposal = ToolCallProposal {
        call_id: ToolCallId::new("tc_round_trip"),
        session_id: SessionId::new("sess_rt"),
        run_id: RunId::new("run_rt"),
        project: ProjectKey::new("default_tenant", "default_workspace", "default_project"),
        tool_name: "read".to_owned(),
        tool_args: json!({ "path": "/tmp/x" }),
        display_summary: Some("read /tmp/x".to_owned()),
        match_policy: ApprovalMatchPolicy::Exact,
    };
    state
        .runtime
        .tool_call_approvals
        .submit_proposal(proposal)
        .await
        .expect("submit");

    // List via the unified surface — must contain the proposal with
    // kind=tool_call.
    let (status, body, _) = call(app.clone(), "GET", "/v1/approvals?run_id=run_rt", None).await;
    assert_eq!(status, StatusCode::OK, "list: {body}");
    let items = body["items"].as_array().expect("items array");
    let hit = items
        .iter()
        .find(|r| r["kind"] == "tool_call" && r["call_id"] == "tc_round_trip")
        .expect("tool-call proposal must appear");
    assert_eq!(hit["state"], "pending");

    // Approve via the unified surface.
    let (status, body, _) = call(
        app.clone(),
        "POST",
        "/v1/approvals/tc_round_trip/approve",
        Some(json!({ "scope": { "type": "once" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "approve: {body}");
    assert_eq!(body["kind"], "tool_call");
    assert_eq!(body["state"], "approved");
    assert_eq!(body["operator_id"], ACTOR);

    // Re-fetch via GET by id confirms the state update.
    let (status, body, _) = call(app, "GET", "/v1/approvals/tc_round_trip", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "approved");
}

// ── 2. Plan-approval round trip via unified surface ────────────────────────

#[tokio::test]
async fn plan_approval_round_trip_via_unified_surface() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    register_principal(&state);

    // Create a plan approval directly via the service. No `run_id`
    // anchor — the fake fabric doesn't seed a run here; the approval
    // service accepts null anchors for release-gate / free-standing
    // approvals.
    let project = ProjectKey::new("default_tenant", "default_workspace", "default_project");
    let approval_id = ApprovalId::new("plan_rt_1");
    state
        .runtime
        .approvals
        .request(
            &project,
            approval_id.clone(),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .expect("request approval");

    // List via the unified surface, narrowing by kind=plan.
    let (status, body, _) = call(app.clone(), "GET", "/v1/approvals?kind=plan", None).await;
    assert_eq!(status, StatusCode::OK, "list plan: {body}");
    let items = body["items"].as_array().expect("items");
    let hit = items
        .iter()
        .find(|r| r["kind"] == "plan" && r["approval_id"] == "plan_rt_1")
        .expect("plan approval surfaces via unified list");
    assert!(hit["decision"].is_null(), "pending");

    // Approve via the unified POST path — body is optional for plan.
    let (status, body, _) =
        call(app.clone(), "POST", "/v1/approvals/plan_rt_1/approve", None).await;
    assert_eq!(status, StatusCode::OK, "plan approve: {body}");
    assert_eq!(body["kind"], "plan");
    assert_eq!(body["decision"], "approved");

    // GET by id returns the resolved record.
    let (status, body, _) = call(app, "GET", "/v1/approvals/plan_rt_1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["kind"], "plan");
    assert_eq!(body["decision"], "approved");
}

// ── 3. Merged list returns both kinds ──────────────────────────────────────

#[tokio::test]
async fn merged_list_returns_both_kinds() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    register_principal(&state);

    // Seed one of each.
    let project = ProjectKey::new("default_tenant", "default_workspace", "default_project");
    state
        .runtime
        .approvals
        .request(
            &project,
            ApprovalId::new("plan_mix_1"),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .expect("plan request");
    let proposal = ToolCallProposal {
        call_id: ToolCallId::new("tc_mix_1"),
        session_id: SessionId::new("sess_mix"),
        run_id: RunId::new("run_mix"),
        project: project.clone(),
        tool_name: "read".to_owned(),
        tool_args: json!({ "path": "/tmp/z" }),
        display_summary: None,
        match_policy: ApprovalMatchPolicy::Exact,
    };
    state
        .runtime
        .tool_call_approvals
        .submit_proposal(proposal)
        .await
        .expect("tc submit");

    let (status, body, _) = call(app, "GET", "/v1/approvals", None).await;
    assert_eq!(status, StatusCode::OK, "merged list: {body}");
    let items = body["items"].as_array().expect("items");
    let kinds: Vec<_> = items.iter().map(|r| r["kind"].clone()).collect();
    assert!(
        kinds.iter().any(|k| k == "plan"),
        "merged list includes plan, got {kinds:?}"
    );
    assert!(
        kinds.iter().any(|k| k == "tool_call"),
        "merged list includes tool_call, got {kinds:?}"
    );
}

// ── 4. Deprecated path redirects ───────────────────────────────────────────

#[tokio::test]
async fn deprecated_list_redirects_to_unified_path() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    register_principal(&state);

    let (status, _body, headers) = call(
        app,
        "GET",
        "/v1/tool-call-approvals?run_id=run_abc&state=pending",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::PERMANENT_REDIRECT,
        "deprecated path must 308",
    );
    let loc = headers.get(header::LOCATION).expect("Location header");
    let loc = loc.to_str().expect("utf-8 location");
    // Query string must be preserved so filters keep working.
    assert!(loc.starts_with("/v1/approvals?"), "redirect target: {loc}");
    assert!(loc.contains("run_id=run_abc"), "query preserved: {loc}");
    assert!(loc.contains("state=pending"), "query preserved: {loc}");
    // Legacy semantics: pre-F45 consumers expect tool-call-only rows.
    // The redirect MUST splice `kind=tool_call` onto the target so a
    // redirect-following client doesn't suddenly receive mixed
    // plan+tool_call rows it can't decode.
    assert!(
        loc.contains("kind=tool_call"),
        "redirect must force kind=tool_call: {loc}",
    );
    // RFC 8594 deprecation signal + RFC 8288 successor link.
    let dep = headers
        .get("deprecation")
        .expect("Deprecation header present on redirect");
    assert_eq!(dep.to_str().unwrap(), "true");
    let link = headers
        .get("link")
        .expect("Link header present on redirect")
        .to_str()
        .unwrap();
    assert!(
        link.contains("rel=\"successor-version\""),
        "Link must advertise successor: {link}"
    );
    assert!(
        link.contains("/v1/approvals"),
        "Link must point at unified path: {link}"
    );
}

#[tokio::test]
async fn deprecated_get_redirect_target_works() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    register_principal(&state);

    let proposal = ToolCallProposal {
        call_id: ToolCallId::new("tc_redir_1"),
        session_id: SessionId::new("sess_r"),
        run_id: RunId::new("run_r"),
        project: ProjectKey::new("default_tenant", "default_workspace", "default_project"),
        tool_name: "read".to_owned(),
        tool_args: json!({ "path": "/tmp/r" }),
        display_summary: None,
        match_policy: ApprovalMatchPolicy::Exact,
    };
    state
        .runtime
        .tool_call_approvals
        .submit_proposal(proposal)
        .await
        .expect("seed");

    // Hit the deprecated GET by id → 308.
    let (status, _body, headers) = call(
        app.clone(),
        "GET",
        "/v1/tool-call-approvals/tc_redir_1",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    let target = headers
        .get(header::LOCATION)
        .expect("Location header")
        .to_str()
        .expect("utf-8")
        .to_owned();
    assert_eq!(target, "/v1/approvals/tc_redir_1");

    // Follow the redirect manually and confirm the target returns
    // the record.
    let (status, body, _) = call(app, "GET", &target, None).await;
    assert_eq!(status, StatusCode::OK, "redirect target works: {body}");
    assert_eq!(body["kind"], "tool_call");
    assert_eq!(body["call_id"], "tc_redir_1");
}

#[tokio::test]
async fn deprecated_approve_redirects_with_308() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    register_principal(&state);

    // 308 must preserve method + body across redirect. We don't follow
    // the redirect in-process here — we just verify the target Location.
    let (status, _body, headers) = call(
        app,
        "POST",
        "/v1/tool-call-approvals/tc_xyz/approve",
        Some(json!({ "scope": { "type": "once" } })),
    )
    .await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    let target = headers
        .get(header::LOCATION)
        .expect("Location")
        .to_str()
        .expect("utf-8");
    assert_eq!(target, "/v1/approvals/tc_xyz/approve");
}

#[tokio::test]
async fn deprecated_list_drops_inbound_kind_and_forces_tool_call() {
    // A pre-F45 client has no reason to send `kind=`, but a confused
    // caller shouldn't be able to flip the redirect target to plan-only.
    // We strip any inbound `kind` before splicing on `kind=tool_call`.
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    register_principal(&state);

    let (status, _body, headers) = call(
        app,
        "GET",
        "/v1/tool-call-approvals?kind=plan&run_id=run_drop",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    let loc = headers
        .get(header::LOCATION)
        .expect("Location")
        .to_str()
        .expect("utf-8");
    assert!(loc.contains("kind=tool_call"), "forced: {loc}");
    assert!(!loc.contains("kind=plan"), "stripped: {loc}");
    assert!(
        loc.contains("run_id=run_drop"),
        "other params preserved: {loc}"
    );
}
