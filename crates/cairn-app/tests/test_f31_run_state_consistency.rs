//! F31 — `/v1/runs` vs `/v1/runs/:id` state-projection consistency.
//!
//! Regression coverage for the dual-entity bug observed live on
//! dogfood-v2-task-1 (2026-04-25): `GET /v1/runs` returned `state=running`
//! while `GET /v1/runs/:id` returned `state=pending, version=0` for the
//! same run. The list endpoint read from the cairn-store `RunReadModel`
//! projection (authoritative, event-sourced) while the detail endpoint
//! went through `runtime.runs.get()` which — under the Fabric adapter —
//! reads FF's `describe_execution` snapshot. FF's snapshot only advances
//! when lifecycle FCALLs fire; any path that emits `RunStateChanged`
//! without calling them leaves the two views disagreeing.
//!
//! Fix: route `load_run_visible_to_tenant` (the helper every single-run
//! handler uses — GET, cancel, intervene, cost-alert, etc.) through
//! `RunReadModel::get(state.runtime.store)` — the exact same source
//! `list_runs_filtered` already uses. One projection, one source of
//! truth. Detail and list can no longer diverge by construction.
//!
//! These tests append `RunCreated` + `RunStateChanged` events directly
//! to `state.runtime.store` (the production event-sourcing path —
//! `apply_projection` runs synchronously inside `store.append`) and then
//! hit both HTTP endpoints, comparing state + version + failure_class.
//! Any regression that re-introduces a separate read path for detail
//! will flunk the cross-endpoint equality assertion.

mod support;

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
    EventEnvelope, EventId, EventSource, FailureClass, ProjectKey, RunCreated, RunId, RunState,
    RunStateChanged, RuntimeEvent, SessionCreated, SessionId, StateTransition,
};
use cairn_store::event_log::EventLog;
use serde_json::Value;

const TOKEN: &str = "f31-consistency-test-token";
const TENANT: &str = "acme";
const WORKSPACE: &str = "prod";
const PROJECT: &str = "minecraft";

fn bearer() -> String {
    format!("Bearer {TOKEN}")
}

fn seed_principal(state: &AppState) {
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: cairn_domain::OperatorId::new("f31_op"),
            tenant: TenantKey::new(TENANT),
        },
    );
}

fn project() -> ProjectKey {
    ProjectKey::new(TENANT, WORKSPACE, PROJECT)
}

fn envelope(id: &str, event: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::System, event)
}

fn run_created_event(session_id: &str, run_id: &str) -> EventEnvelope<RuntimeEvent> {
    envelope(
        &format!("f31_rc_{run_id}"),
        RuntimeEvent::RunCreated(RunCreated {
            project: project(),
            session_id: SessionId::new(session_id),
            run_id: RunId::new(run_id),
            parent_run_id: None,
            prompt_release_id: None,
            agent_role_id: None,
        }),
    )
}

fn session_created_event(session_id: &str) -> EventEnvelope<RuntimeEvent> {
    envelope(
        &format!("f31_sc_{session_id}"),
        RuntimeEvent::SessionCreated(SessionCreated {
            project: project(),
            session_id: SessionId::new(session_id),
        }),
    )
}

fn run_state_changed(
    run_id: &str,
    from: Option<RunState>,
    to: RunState,
    failure_class: Option<FailureClass>,
) -> EventEnvelope<RuntimeEvent> {
    envelope(
        &format!("f31_rsc_{run_id}_{to:?}"),
        RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project(),
            run_id: RunId::new(run_id),
            transition: StateTransition { from, to },
            failure_class,
            pause_reason: None,
            resume_trigger: None,
        }),
    )
}

async fn http_get(app: Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", bearer())
        .body(Body::empty())
        .unwrap();
    let res = tower::ServiceExt::oneshot(app, req).await.unwrap();
    let status = res.status();
    // 10 MB body cap — well above any realistic run-detail / run-list response
    // and keeps the test from DoSing itself if a handler ever regresses into
    // unbounded output.
    let bytes = to_bytes(res.into_body(), 10 * 1024 * 1024).await.unwrap();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

fn find_run<'a>(list_body: &'a Value, run_id: &str) -> &'a Value {
    let items = list_body["items"]
        .as_array()
        .unwrap_or_else(|| panic!("list body missing items: {list_body}"));
    items
        .iter()
        .find(|r| r["run_id"] == run_id)
        .unwrap_or_else(|| panic!("run {run_id} not found in list: {list_body}"))
}

/// The run payload on `GET /v1/runs/:id` is wrapped in `{ run: ..., tasks: [] }`.
/// `list_runs_filtered` returns the unwrapped record. Comparing requires
/// pulling the inner record out.
fn detail_run(detail_body: &Value) -> &Value {
    detail_body
        .get("run")
        .unwrap_or_else(|| panic!("detail body missing 'run' field: {detail_body}"))
}

/// Drives the exact dual-read cross-check: same scope, same run, both
/// endpoints must return byte-identical run-state projection fields.
/// These are the fields the RunDetailPage and RunsPage bind to; if any of
/// them drifts, operator UX silently lies about run progress.
fn assert_list_and_detail_agree(list_body: &Value, detail_body: &Value, run_id: &str) {
    let list_run = find_run(list_body, run_id);
    let detail = detail_run(detail_body);

    for field in [
        "run_id",
        "session_id",
        "state",
        "version",
        "failure_class",
        "pause_reason",
        "resume_trigger",
        "updated_at",
    ] {
        assert_eq!(
            list_run[field], detail[field],
            "field `{field}` diverged between list and detail for run {run_id}\n  list  = {list_run}\n  detail= {detail}",
        );
    }
}

/// Baseline: after run-creation, pending state + version=1 on both
/// endpoints. This is the pre-orchestration state the UI shows on the
/// runs dashboard before anyone clicks "start".
#[tokio::test]
async fn list_and_detail_agree_on_pending_state() {
    let (router, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal(&state);

    let session_id = "sess_pending";
    let run_id = "run_pending";

    state
        .runtime
        .store
        .append(&[
            session_created_event(session_id),
            run_created_event(session_id, run_id),
        ])
        .await
        .expect("append");

    let (list_status, list_body) = http_get(
        router.clone(),
        &format!("/v1/runs?tenant_id={TENANT}&workspace_id={WORKSPACE}&project_id={PROJECT}"),
    )
    .await;
    assert_eq!(list_status, StatusCode::OK, "list 200: {list_body}");

    let (detail_status, detail_body) =
        http_get(router.clone(), &format!("/v1/runs/{run_id}")).await;
    assert_eq!(detail_status, StatusCode::OK, "detail 200: {detail_body}");

    // Pre-fix baseline: both report pending, version=1.
    let list_run = find_run(&list_body, run_id);
    assert_eq!(list_run["state"], "pending");
    assert_eq!(list_run["version"], 1);

    let detail = detail_run(&detail_body);
    assert_eq!(detail["state"], "pending");
    assert_eq!(detail["version"], 1);

    assert_list_and_detail_agree(&list_body, &detail_body, run_id);
}

/// Transitions pending → running → succeeded and asserts list + detail
/// agree at every hop. Before F31, detail would stay stuck at the initial
/// value (FF snapshot wasn't transitioned by the projection-only event
/// append) while list caught each transition from the projection.
#[tokio::test]
async fn list_and_detail_agree_through_full_lifecycle() {
    let (router, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal(&state);

    let session_id = "sess_full";
    let run_id = "run_full";

    state
        .runtime
        .store
        .append(&[
            session_created_event(session_id),
            run_created_event(session_id, run_id),
        ])
        .await
        .expect("append session+run");

    // Drive pending → running.
    state
        .runtime
        .store
        .append(&[run_state_changed(
            run_id,
            Some(RunState::Pending),
            RunState::Running,
            None,
        )])
        .await
        .expect("append running transition");

    let (_, list_body) = http_get(
        router.clone(),
        &format!("/v1/runs?tenant_id={TENANT}&workspace_id={WORKSPACE}&project_id={PROJECT}"),
    )
    .await;
    let (_, detail_body) = http_get(router.clone(), &format!("/v1/runs/{run_id}")).await;

    let list_run = find_run(&list_body, run_id);
    assert_eq!(list_run["state"], "running", "list running: {list_body}");
    assert_eq!(list_run["version"], 2);
    assert_list_and_detail_agree(&list_body, &detail_body, run_id);

    // Drive running → completed.
    state
        .runtime
        .store
        .append(&[run_state_changed(
            run_id,
            Some(RunState::Running),
            RunState::Completed,
            None,
        )])
        .await
        .expect("append completed transition");

    let (_, list_body) = http_get(
        router.clone(),
        &format!("/v1/runs?tenant_id={TENANT}&workspace_id={WORKSPACE}&project_id={PROJECT}"),
    )
    .await;
    let (_, detail_body) = http_get(router.clone(), &format!("/v1/runs/{run_id}")).await;

    let list_run = find_run(&list_body, run_id);
    assert_eq!(list_run["state"], "completed");
    assert_eq!(list_run["version"], 3);
    assert_list_and_detail_agree(&list_body, &detail_body, run_id);
}

/// Terminal-failure transition carries a `failure_class`; both endpoints
/// must surface it. Regression coverage for the RunDetailPage's failure
/// banner vs the RunsPage's red-row indicator staying in sync.
#[tokio::test]
async fn list_and_detail_agree_on_failed_run_with_failure_class() {
    let (router, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal(&state);

    let session_id = "sess_fail";
    let run_id = "run_fail";

    state
        .runtime
        .store
        .append(&[
            session_created_event(session_id),
            run_created_event(session_id, run_id),
            run_state_changed(run_id, Some(RunState::Pending), RunState::Running, None),
            run_state_changed(
                run_id,
                Some(RunState::Running),
                RunState::Failed,
                Some(FailureClass::ExecutionError),
            ),
        ])
        .await
        .expect("append lifecycle");

    let (list_status, list_body) = http_get(
        router.clone(),
        &format!("/v1/runs?tenant_id={TENANT}&workspace_id={WORKSPACE}&project_id={PROJECT}"),
    )
    .await;
    assert_eq!(list_status, StatusCode::OK);

    let (detail_status, detail_body) =
        http_get(router.clone(), &format!("/v1/runs/{run_id}")).await;
    assert_eq!(detail_status, StatusCode::OK);

    let list_run = find_run(&list_body, run_id);
    assert_eq!(list_run["state"], "failed");
    let detail = detail_run(&detail_body);
    assert_eq!(detail["state"], "failed");
    // `failure_class` must be populated — RunDetailPage renders the
    // failure banner off this field.
    assert!(
        !detail["failure_class"].is_null(),
        "failure_class must not be null on a failed run: {detail_body}"
    );
    assert_list_and_detail_agree(&list_body, &detail_body, run_id);
}

/// Canceled runs. The orchestrator-level cancel path emits
/// `RunStateChanged(to=Canceled)`; UI must show canceled on both views.
#[tokio::test]
async fn list_and_detail_agree_on_canceled_run() {
    let (router, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    seed_principal(&state);

    let session_id = "sess_cancel";
    let run_id = "run_cancel";

    state
        .runtime
        .store
        .append(&[
            session_created_event(session_id),
            run_created_event(session_id, run_id),
            run_state_changed(run_id, Some(RunState::Pending), RunState::Running, None),
            run_state_changed(run_id, Some(RunState::Running), RunState::Canceled, None),
        ])
        .await
        .expect("append lifecycle");

    let (_, list_body) = http_get(
        router.clone(),
        &format!("/v1/runs?tenant_id={TENANT}&workspace_id={WORKSPACE}&project_id={PROJECT}"),
    )
    .await;
    let (_, detail_body) = http_get(router.clone(), &format!("/v1/runs/{run_id}")).await;

    let list_run = find_run(&list_body, run_id);
    assert_eq!(list_run["state"], "canceled");
    let detail = detail_run(&detail_body);
    assert_eq!(detail["state"], "canceled");
    assert_list_and_detail_agree(&list_body, &detail_body, run_id);
}

/// Cross-tenant detail lookup returns 404, matching the list scope-filter.
/// Guards against the helper accidentally leaking a different tenant's
/// run now that it reads straight from the projection.
#[tokio::test]
async fn detail_404s_for_cross_tenant_access() {
    let (router, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    // Principal bound to a DIFFERENT tenant than the run.
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: cairn_domain::OperatorId::new("f31_intruder"),
            tenant: TenantKey::new("other-tenant"),
        },
    );

    let session_id = "sess_crosstenant";
    let run_id = "run_crosstenant";

    state
        .runtime
        .store
        .append(&[
            session_created_event(session_id),
            run_created_event(session_id, run_id),
        ])
        .await
        .expect("append");

    let (detail_status, _) = http_get(router.clone(), &format!("/v1/runs/{run_id}")).await;
    assert_eq!(
        detail_status,
        StatusCode::NOT_FOUND,
        "cross-tenant detail must 404, not leak"
    );
}
