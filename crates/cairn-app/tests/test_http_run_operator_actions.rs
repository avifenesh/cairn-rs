//! Regression coverage for UI operator-mutation actions (issues #166 + #173).
//!
//! Verifies the end-to-end HTTP contract for the run mutation endpoints
//! that the RunDetailPage / OrchestrationPage now expose to operators:
//!
//!   * POST /v1/runs/:id/pause      → state transitions to `paused`
//!   * POST /v1/runs/:id/resume     → state transitions back to `running`
//!   * POST /v1/runs/:id/spawn      → child appears in
//!     `GET /v1/runs/:id/children`
//!   * POST /v1/runs/:id/intervene  → intervention shows up in
//!     `GET /v1/runs/:id/interventions`
//!
//! This is a HTTP contract test — state transitions are driven solely by
//! the runtime's `RunService`, so exercising the endpoints is sufficient
//! to catch UI drift (missing/renamed struct fields, 404s, etc.).

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

// Run mutation handlers that don't bypass tenant scope for admin tokens
// (`spawn_subagent_run_handler`, `intervene_run_handler`) require the
// token's implicit scope to match the run's `project.tenant_id`. Bind all
// tests to the default admin scope — same pattern used by
// `test_http_provider_lifecycle.rs`.
// The admin service account is registered with tenant id `"default"` in
// `main.rs`. Spawn and intervene handlers compare the run's
// `project.tenant_id` to this principal's tenant without an admin bypass,
// so tests must create runs under the same tenant.
const TENANT: &str = "default";
const WORKSPACE: &str = "default_workspace";
const PROJECT: &str = "default_project";

/// Create a session and a run under the default admin scope, returning
/// `(session_id, run_id)`.
async fn create_session_and_run(h: &LiveHarness, label: &str) -> (String, String) {
    let suffix = &h.project; // already uuid-unique per harness
    let session_id = format!("sess_{label}_{suffix}");
    let run_id = format!("run_{label}_{suffix}");

    let res = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .header("X-Cairn-Tenant", TENANT)
        .header("X-Cairn-Workspace", WORKSPACE)
        .header("X-Cairn-Project", PROJECT)
        .json(&json!({
            "tenant_id": TENANT,
            "workspace_id": WORKSPACE,
            "project_id": PROJECT,
            "session_id": session_id,
        }))
        .send()
        .await
        .expect("POST /v1/sessions reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "session create: {}",
        res.text().await.unwrap_or_default(),
    );

    let res = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .header("X-Cairn-Tenant", TENANT)
        .header("X-Cairn-Workspace", WORKSPACE)
        .header("X-Cairn-Project", PROJECT)
        .json(&json!({
            "tenant_id": TENANT,
            "workspace_id": WORKSPACE,
            "project_id": PROJECT,
            "session_id": session_id,
            "run_id": run_id,
        }))
        .send()
        .await
        .expect("POST /v1/runs reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "run create: {}",
        res.text().await.unwrap_or_default(),
    );

    // Wait for the run projection to settle before exercising mutation
    // handlers that look the run up via `runs.get()` (no event-log
    // fallback). Poll `GET /v1/runs/:id` — it uses the projection path
    // too, so once it returns 200 every other handler will find the run.
    let mut found = false;
    let mut last_status = 0u16;
    let mut last_body = String::new();
    for _ in 0..40 {
        let res = h
            .client()
            .get(format!("{}/v1/runs/{}", h.base_url, run_id))
            .bearer_auth(&h.admin_token)
            .header("X-Cairn-Tenant", TENANT)
            .header("X-Cairn-Workspace", WORKSPACE)
            .header("X-Cairn-Project", PROJECT)
            .send()
            .await
            .expect("GET /v1/runs/:id reaches server");
        last_status = res.status().as_u16();
        if last_status == 200 {
            found = true;
            break;
        }
        last_body = res.text().await.unwrap_or_default();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        found,
        "run {run_id} not visible via GET /v1/runs/:id after 4s; last_status={last_status}, body={last_body}",
    );

    (session_id, run_id)
}

/// Extract the `run` field from handlers that respond with `RunRecord`
/// either directly or wrapped in `{ run: ... }`.
fn unwrap_run(body: Value) -> Value {
    if body.get("run_id").is_some() {
        body
    } else if let Some(r) = body.get("run").cloned() {
        r
    } else {
        panic!("expected run body, got {body}")
    }
}

/// Contract test: pause and resume endpoints are wired end-to-end, accept
/// the documented request shape, and do not 404 (which would indicate the
/// route disappeared or the handler can no longer resolve the run).
///
/// Pause of a pending (not-yet-claimed) run bottoms out in FF's
/// `ff_suspend_execution`, which only pauses a claimed execution, so a
/// pause request here may legitimately return a 4xx/5xx from fabric. The
/// contract the UI depends on is: the route exists, the request body
/// round-trips, and the response is shaped as either a `RunRecord` or a
/// `{code, message}` error envelope. That's what this test pins.
///
/// The full transition semantics (pending → paused → running) are
/// covered at the fabric crate level in
/// `crates/cairn-fabric/tests/integration/test_suspension.rs`.
#[tokio::test]
async fn pause_and_resume_endpoints_are_wired() {
    let h = LiveHarness::setup().await;
    let (_session_id, run_id) = create_session_and_run(&h, "pause").await;

    // Pause — must not 404 (route/handler wiring) and must return a JSON
    // envelope we can decode. If 200, it must carry `state: paused`.
    let res = h
        .client()
        .post(format!("{}/v1/runs/{}/pause", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .header("X-Cairn-Tenant", TENANT)
        .header("X-Cairn-Workspace", WORKSPACE)
        .header("X-Cairn-Project", PROJECT)
        .json(&json!({ "reason_kind": "operator_pause", "actor": "operator-test" }))
        .send()
        .await
        .expect("POST /v1/runs/:id/pause reaches server");
    let status = res.status().as_u16();
    assert_ne!(status, 404, "pause endpoint must be wired, got 404");
    let body: Value = res.json().await.expect("pause body is json");
    if status == 200 {
        let paused = unwrap_run(body.clone());
        assert_eq!(
            paused.get("state").and_then(|s| s.as_str()),
            Some("paused"),
            "when pause returns 200, state must be `paused`: {paused:?}",
        );
    } else {
        assert!(
            body.get("code").is_some() || body.get("message").is_some(),
            "non-200 pause must return error envelope with code/message: {body:?}",
        );
    }

    // Resume — same contract shape. If 200, state must be `running` or
    // `pending` per `RunResumeTarget`.
    let res = h
        .client()
        .post(format!("{}/v1/runs/{}/resume", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .header("X-Cairn-Tenant", TENANT)
        .header("X-Cairn-Workspace", WORKSPACE)
        .header("X-Cairn-Project", PROJECT)
        .json(&json!({ "trigger": "operator_resume", "target": "running" }))
        .send()
        .await
        .expect("POST /v1/runs/:id/resume reaches server");
    let status = res.status().as_u16();
    assert_ne!(status, 404, "resume endpoint must be wired, got 404");
    let body: Value = res.json().await.expect("resume body is json");
    if status == 200 {
        let resumed = unwrap_run(body.clone());
        let state = resumed.get("state").and_then(|s| s.as_str()).unwrap_or("");
        assert!(
            state == "running" || state == "pending",
            "when resume returns 200, state must be `running` or `pending`: {resumed:?}",
        );
    } else {
        assert!(
            body.get("code").is_some() || body.get("message").is_some(),
            "non-200 resume must return error envelope with code/message: {body:?}",
        );
    }
}

#[tokio::test]
async fn spawn_subagent_appears_in_children_list() {
    let h = LiveHarness::setup().await;
    let (session_id, run_id) = create_session_and_run(&h, "spawn").await;

    // Spawn under the same session.
    let child_run_id = format!("run_child_{}", &h.project);
    let res = h
        .client()
        .post(format!("{}/v1/runs/{}/spawn", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .header("X-Cairn-Tenant", TENANT)
        .header("X-Cairn-Workspace", WORKSPACE)
        .header("X-Cairn-Project", PROJECT)
        .json(&json!({
            "session_id": session_id,
            "child_run_id": child_run_id,
        }))
        .send()
        .await
        .expect("POST /v1/runs/:id/spawn reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "spawn: {}",
        res.text().await.unwrap_or_default(),
    );
    let spawn_body: Value = res.json().await.expect("spawn json");
    assert_eq!(
        spawn_body.get("child_run_id").and_then(|s| s.as_str()),
        Some(child_run_id.as_str()),
        "spawn response must echo the child_run_id",
    );

    // Children list must include the spawned child.
    let res = h
        .client()
        .get(format!("{}/v1/runs/{}/children?limit=50", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .header("X-Cairn-Tenant", TENANT)
        .header("X-Cairn-Workspace", WORKSPACE)
        .header("X-Cairn-Project", PROJECT)
        .send()
        .await
        .expect("GET /v1/runs/:id/children reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "list children: {}",
        res.text().await.unwrap_or_default(),
    );
    let body: Value = res.json().await.expect("children json");
    let items = body
        .as_array()
        .cloned()
        .or_else(|| body.get("items").and_then(|v| v.as_array()).cloned())
        .expect("children body is array or {items: [..]}");
    let child_ids: Vec<&str> = items
        .iter()
        .filter_map(|v| v.get("run_id").and_then(|x| x.as_str()))
        .collect();
    assert!(
        child_ids.contains(&child_run_id.as_str()),
        "child {child_run_id} missing from children list: {child_ids:?}",
    );
}

#[tokio::test]
async fn intervene_records_intervention() {
    let h = LiveHarness::setup().await;
    let (_session_id, run_id) = create_session_and_run(&h, "inter").await;

    let res = h
        .client()
        .post(format!("{}/v1/runs/{}/intervene", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .header("X-Cairn-Tenant", TENANT)
        .header("X-Cairn-Workspace", WORKSPACE)
        .header("X-Cairn-Project", PROJECT)
        .json(&json!({
            "action": "inject_message",
            "reason": "operator checking run",
            "message_body": "hello from test",
        }))
        .send()
        .await
        .expect("POST /v1/runs/:id/intervene reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "intervene: {}",
        res.text().await.unwrap_or_default(),
    );
    let body: Value = res.json().await.expect("intervene json");
    assert_eq!(
        body.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "intervene must return ok=true, got {body:?}",
    );

    // List interventions must contain our entry.
    let res = h
        .client()
        .get(format!("{}/v1/runs/{}/interventions?limit=50", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .header("X-Cairn-Tenant", TENANT)
        .header("X-Cairn-Workspace", WORKSPACE)
        .header("X-Cairn-Project", PROJECT)
        .send()
        .await
        .expect("GET /v1/runs/:id/interventions reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "list interventions: {}",
        res.text().await.unwrap_or_default(),
    );
    let body: Value = res.json().await.expect("interventions json");
    let items = body
        .as_array()
        .cloned()
        .or_else(|| body.get("items").and_then(|v| v.as_array()).cloned())
        .expect("interventions body is array or {items: [..]}");
    assert!(
        !items.is_empty(),
        "interventions list must not be empty after POST /intervene",
    );
    let actions: Vec<&str> = items
        .iter()
        .filter_map(|v| v.get("action").and_then(|x| x.as_str()))
        .collect();
    assert!(
        actions.iter().any(|a| a.contains("message") || a.contains("inject")),
        "inject_message action must be recorded: got {actions:?}",
    );
}
