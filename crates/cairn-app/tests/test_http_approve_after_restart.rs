//! F22 dogfood reproducer: approve-after-restart.
//!
//! In production, the sequence is:
//!
//! 1. Orchestrator submits a `ToolCallProposal` through
//!    `POST /v1/runs/:id/orchestrate` → `ToolCallProposed` event is
//!    appended to the durable event log *and* a `ProposalEntry` lands in
//!    the in-memory cache.
//! 2. Operator restarts `cairn-app` (or fails over to another replica).
//!    The projection still carries the pending approval, but the new
//!    process starts with an empty in-memory cache.
//! 3. Operator calls `POST /v1/tool-call-approvals/:call_id/approve`.
//!    The HTTP handler's `load_record_visible_to_tenant` lookup hits the
//!    projection and succeeds — proving the approval exists — and then
//!    hands off to `ToolCallApprovalService::approve`.
//!
//! Before the F22 fix, step 3 returned `404 tool_call_approval not found`
//! because the service's in-memory cache had no entry. This test walks
//! the same surface: submit through the HTTP router, then drive approve
//! / reject / amend against a **fresh** `ToolCallApprovalServiceImpl`
//! over the same `InMemoryStore`. The fresh service has an empty cache
//! — the exact post-restart state — and must re-hydrate from the
//! projection.
//!
//! (The cairn-app bootstrap seeds a default tenant on construction, so
//! building a second router on the same store conflicts. Instead we
//! construct a bare `ToolCallApprovalServiceImpl` directly — this is
//! exactly the object the HTTP handler ends up invoking anyway.)

mod support;

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use cairn_api::auth::AuthPrincipal;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_app::AppState;
use cairn_domain::tenancy::TenantKey;
use cairn_domain::{
    ApprovalMatchPolicy, ApprovalScope, OperatorId, ProjectKey, RunId, SessionId, ToolCallId,
};
use cairn_runtime::services::{ToolCallApprovalReaderAdapter, ToolCallApprovalServiceImpl};
use cairn_runtime::tool_call_approvals::{ToolCallApprovalService, ToolCallProposal};
use cairn_store::projections::{ToolCallApprovalReadModel, ToolCallApprovalState};
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "restart-test-token";
const ACTOR: &str = "restart_op";

fn bearer() -> String {
    format!("Bearer {TOKEN}")
}

fn seed_principal(state: &AppState) {
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new(ACTOR),
            tenant: TenantKey::new("default_tenant"),
        },
    );
}

async fn http_json(
    app: Router,
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

fn proposal_at(call_id: &str) -> ToolCallProposal {
    ToolCallProposal {
        call_id: ToolCallId::new(call_id),
        session_id: SessionId::new("sess_restart"),
        run_id: RunId::new("run_restart"),
        project: ProjectKey::new("default_tenant", "default_workspace", "default_project"),
        tool_name: "read".to_owned(),
        tool_args: json!({ "path": "/tmp/restart.txt" }),
        display_summary: Some("read /tmp/restart.txt".to_owned()),
        match_policy: ApprovalMatchPolicy::Exact,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn approve_succeeds_after_service_restart_with_same_store() {
    // 1. Boot router A, seed a proposal through the real HTTP-invoking
    //    service — this populates the durable event log AND the
    //    in-memory cache on service A.
    let (router_a, state_a) =
        support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal(&state_a);
    state_a
        .runtime
        .tool_call_approvals
        .submit_proposal(proposal_at("tc_restart"))
        .await
        .expect("submit");

    // Sanity: HTTP GET on router A sees the pending record (projection
    // is populated).
    let (get_status, get_body) =
        http_json(router_a, "GET", "/v1/tool-call-approvals/tc_restart", None).await;
    assert_eq!(get_status, StatusCode::OK, "GET pre-restart: {get_body}");
    assert_eq!(get_body["state"], "pending");

    // 2. Simulate restart: build a fresh `ToolCallApprovalServiceImpl`
    //    over the same store. Its in-memory cache is empty — same world
    //    a new process sees after failover.
    let store = state_a.runtime.store.clone();
    // Wire the reader through ToolCallApprovalReaderAdapter to match
    // production wiring (AppState::boot uses the adapter). This keeps
    // get_tool_call_approval's state-filter semantics under test
    // rather than relying on the blanket impl's default behaviour.
    let reader = Arc::new(ToolCallApprovalReaderAdapter::new(store.clone()));
    let service_b = ToolCallApprovalServiceImpl::new(store.clone(), reader);

    // 3. Core repro: approve via service B, whose cache has no entry for
    //    tc_restart. Before the F22 fix this returned NotFound (→ HTTP
    //    404). After the fix it rehydrates from the projection.
    service_b
        .approve(
            ToolCallId::new("tc_restart"),
            OperatorId::new(ACTOR),
            ApprovalScope::Once,
            None,
        )
        .await
        .expect("approve after restart must NOT be NotFound");

    // 4. Projection reflects the approval.
    let reader: &dyn ToolCallApprovalReadModel = store.as_ref();
    let record = reader
        .get(&ToolCallId::new("tc_restart"))
        .await
        .unwrap()
        .expect("record persisted");
    assert_eq!(record.state, ToolCallApprovalState::Approved);
    assert_eq!(record.operator_id, Some(OperatorId::new(ACTOR)));
}

#[tokio::test]
async fn reject_succeeds_after_service_restart_with_same_store() {
    let (_router_a, state_a) =
        support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal(&state_a);
    state_a
        .runtime
        .tool_call_approvals
        .submit_proposal(proposal_at("tc_reject_restart"))
        .await
        .expect("submit");

    let store = state_a.runtime.store.clone();
    // Wire the reader through ToolCallApprovalReaderAdapter to match
    // production wiring (AppState::boot uses the adapter). This keeps
    // get_tool_call_approval's state-filter semantics under test
    // rather than relying on the blanket impl's default behaviour.
    let reader = Arc::new(ToolCallApprovalReaderAdapter::new(store.clone()));
    let service_b = ToolCallApprovalServiceImpl::new(store.clone(), reader);

    service_b
        .reject(
            ToolCallId::new("tc_reject_restart"),
            OperatorId::new(ACTOR),
            Some("not now".into()),
        )
        .await
        .expect("reject after restart must NOT be NotFound");

    let reader: &dyn ToolCallApprovalReadModel = store.as_ref();
    let record = reader
        .get(&ToolCallId::new("tc_reject_restart"))
        .await
        .unwrap()
        .expect("record persisted");
    assert_eq!(record.state, ToolCallApprovalState::Rejected);
    assert_eq!(record.reason.as_deref(), Some("not now"));
}

#[tokio::test]
async fn amend_succeeds_after_service_restart_with_same_store() {
    let (_router_a, state_a) =
        support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal(&state_a);
    state_a
        .runtime
        .tool_call_approvals
        .submit_proposal(proposal_at("tc_amend_restart"))
        .await
        .expect("submit");

    let store = state_a.runtime.store.clone();
    // Wire the reader through ToolCallApprovalReaderAdapter to match
    // production wiring (AppState::boot uses the adapter). This keeps
    // get_tool_call_approval's state-filter semantics under test
    // rather than relying on the blanket impl's default behaviour.
    let reader = Arc::new(ToolCallApprovalReaderAdapter::new(store.clone()));
    let service_b = ToolCallApprovalServiceImpl::new(store.clone(), reader);

    service_b
        .amend(
            ToolCallId::new("tc_amend_restart"),
            OperatorId::new(ACTOR),
            json!({ "path": "/tmp/amended.txt" }),
        )
        .await
        .expect("amend after restart must NOT be NotFound");

    let reader: &dyn ToolCallApprovalReadModel = store.as_ref();
    let record = reader
        .get(&ToolCallId::new("tc_amend_restart"))
        .await
        .unwrap()
        .expect("record persisted");
    // Amend leaves state pending but updates amended_tool_args.
    assert_eq!(record.state, ToolCallApprovalState::Pending);
    assert_eq!(
        record.amended_tool_args,
        Some(json!({ "path": "/tmp/amended.txt" }))
    );
}
