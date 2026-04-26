//! F47 PR2 — event-sourced persistence of run completion annotation.
//!
//! PR1 (cc8f90d5) made the `CompletionVerification` sidecar visible on the
//! SSE `orchestrate_finished` frame so live operators could cross-check the
//! LLM's free-text summary against warning / error lines distilled from
//! tool_result frames. After an SSE disconnect or a browser refresh that
//! block vanished. PR2 closes that gap: a new `RunCompletionAnnotated`
//! domain event lands on the event log after the run terminates, three
//! backend projections (in-memory, Postgres, SQLite) store it on the
//! existing `runs` row via nullable `completion_summary` +
//! `completion_verification_json` + `completion_annotated_at_ms` columns,
//! and `GET /v1/runs/:id` returns a `completion: { summary, verification,
//! completed_at }` object so the evidence survives the stream.
//!
//! These tests exercise the full wire-level contract end-to-end against a
//! live cairn-app subprocess (`LiveHarness`), per the integration-tests-
//! only memory. No mocks of the store, projection, or HTTP handler.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

const MODEL_ID: &str = "openrouter/f47-pr2-persistence";

#[derive(Clone)]
struct MockState {
    hits: Arc<AtomicUsize>,
    marker: String,
    claimed_summary: String,
}

/// Two-round scripted provider:
///   round 0 → invoke_tool bash, stdout carries `warning: <marker>`
///   round 1+ → complete_run with an overclaim (`claimed_summary`) so the
///              test proves the persisted verification contradicts it.
async fn spawn_mock(marker: String, claimed_summary: String) -> (String, Arc<AtomicUsize>) {
    let state = MockState {
        hits: Arc::new(AtomicUsize::new(0)),
        marker: marker.clone(),
        claimed_summary,
    };
    let hits = state.hits.clone();

    async fn chat_handler(
        State(state): State<MockState>,
        Json(_body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        let n = state.hits.fetch_add(1, Ordering::SeqCst);
        let content = if n == 0 {
            json!([{
                "action_type":      "invoke_tool",
                "description":      "emit a marker warning via bash",
                "tool_name":        "bash",
                "tool_args":        {
                    "command": format!("printf '%s\\n' 'warning: {}'", state.marker),
                },
                "confidence":       0.95,
                "requires_approval": false,
            }])
        } else {
            json!([{
                "action_type":      "complete_run",
                "description":      state.claimed_summary,
                "confidence":       0.95,
                "requires_approval": false,
            }])
        };

        (
            StatusCode::OK,
            Json(json!({
                "id": format!("chat_{n}"),
                "object": "chat.completion",
                "created": 0,
                "model": MODEL_ID,
                "choices": [{
                    "index": 0,
                    "message": {
                        "role":    "assistant",
                        "content": content.to_string(),
                    },
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens":     10,
                    "completion_tokens": 10,
                    "total_tokens":      20,
                },
            })),
        )
    }

    let app = Router::new()
        .route("/chat/completions", post(chat_handler))
        .route("/v1/chat/completions", post(chat_handler))
        .route(
            "/v1/models",
            get(|| async { Json(json!({ "data": [{ "id": MODEL_ID }] })) }),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    (format!("http://{addr}"), hits)
}

/// Drive a run end-to-end against `h` and return the run_id.
///
/// Shared by the in-memory and SQLite tests so both backends exercise the
/// exact same wire flow — the only difference is which store backend
/// LiveHarness spun up.
async fn drive_run_to_completed(
    h: &LiveHarness,
    marker: &str,
    claimed_summary: &str,
) -> (String, String, String) {
    let scope_suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_f47pr2_{scope_suffix}");
    let session_id = format!("sess_f47pr2_{scope_suffix}");
    let run_id = format!("run_f47pr2_{scope_suffix}");

    let (mock_url, _hits) = spawn_mock(marker.to_owned(), claimed_summary.to_owned()).await;

    // Credential
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id":     "openrouter",
            "plaintext_value": format!("sk-f47pr2-{scope_suffix}"),
        }))
        .send()
        .await
        .expect("credential");
    assert_eq!(r.status().as_u16(), 201);
    let credential_id = r
        .json::<Value>()
        .await
        .unwrap()
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_owned();

    // Connection
    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":               tenant,
            "provider_connection_id":  connection_id,
            "provider_family":         "openrouter",
            "adapter_type":            "openrouter",
            "supported_models":        [MODEL_ID],
            "credential_id":           credential_id,
            "endpoint_url":            mock_url,
        }))
        .send()
        .await
        .expect("connection");
    assert_eq!(
        r.status().as_u16(),
        201,
        "connection: {}",
        r.text().await.unwrap_or_default()
    );

    for key in ["generate_model", "brain_model"] {
        let r = h
            .client()
            .put(format!(
                "{}/v1/settings/defaults/system/system/{}",
                h.base_url, key,
            ))
            .bearer_auth(&h.admin_token)
            .json(&json!({ "value": MODEL_ID }))
            .send()
            .await
            .expect("defaults");
        assert_eq!(r.status().as_u16(), 200);
    }

    // Session
    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    tenant,
            "workspace_id": workspace,
            "project_id":   project,
            "session_id":   session_id,
        }))
        .send()
        .await
        .expect("session");
    assert_eq!(r.status().as_u16(), 201);

    // Run
    let r = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    tenant,
            "workspace_id": workspace,
            "project_id":   project,
            "session_id":   session_id,
            "run_id":       run_id,
        }))
        .send()
        .await
        .expect("run");
    assert_eq!(r.status().as_u16(), 201);

    // Orchestrate
    let orch = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal":           "emit a warning then call complete_run",
            "max_iterations": 4,
        }))
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .expect("orchestrate");
    let orch_status = orch.status().as_u16();
    let orch_body: Value = orch.json().await.unwrap_or(Value::Null);
    assert_eq!(
        orch_status, 200,
        "orchestrate must 200 on Completed: body={orch_body}"
    );
    assert_eq!(
        orch_body.get("termination").and_then(|v| v.as_str()),
        Some("completed"),
        "run must terminate Completed for annotation to be persisted: {orch_body}"
    );

    (run_id, session_id, tenant)
}

/// Helper: GET /v1/runs/:id and assert the response status + return body.
async fn fetch_run(h: &LiveHarness, run_id: &str) -> (u16, Value) {
    let r = h
        .client()
        .get(format!("{}/v1/runs/{}", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET run");
    let status = r.status().as_u16();
    let body: Value = r.json().await.unwrap_or(Value::Null);
    (status, body)
}

// ── Test A: in-memory store ─────────────────────────────────────────────────
//
// Drives a run to Completed with a known warning and an overclaiming
// summary, then asserts the persisted `completion` object on
// `GET /v1/runs/:id` carries both halves — the free-text claim AND the
// extractor's contradicting warning.

#[tokio::test]
async fn run_detail_carries_completion_summary_and_verification_in_memory() {
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    let marker = format!("f47pr2-imem-warn-{suffix}");
    let claim = format!("all checks passed cleanly {suffix}");

    let h = LiveHarness::setup().await;
    let (run_id, _session_id, _tenant) = drive_run_to_completed(&h, &marker, &claim).await;

    // Poll briefly: the RunCompletionAnnotated append is fire-and-forget on
    // the orchestrate response path; the projection should land well within
    // 5 s but we give some slack to avoid flakes on loaded CI.
    let mut body = Value::Null;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let (status, b) = fetch_run(&h, &run_id).await;
        assert_eq!(status, 200, "GET /v1/runs/{run_id} must 200: {b}");
        if b.get("completion").is_some() {
            body = b;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let completion = body
        .get("completion")
        .unwrap_or_else(|| panic!("F47 PR2: `completion` missing after polling. body={body}"));

    assert_eq!(
        completion.get("summary").and_then(Value::as_str),
        Some(claim.as_str()),
        "summary must equal the LLM's overclaim verbatim: {completion}"
    );
    let verification = completion
        .get("verification")
        .unwrap_or_else(|| panic!("verification missing: {completion}"));
    let warnings = verification
        .get("warnings")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("warnings array missing: {verification}"));
    assert!(
        warnings
            .iter()
            .filter_map(Value::as_str)
            .any(|w| w.contains(&marker)),
        "marker {marker:?} must surface in persisted `warnings` — otherwise \
         the truth-vs-claim gap is not visible post-refresh: {verification}"
    );
    assert_eq!(
        verification
            .get("extractor_version")
            .and_then(Value::as_u64),
        Some(1),
        "extractor_version stamp lost on projection roundtrip: {verification}"
    );
    assert!(
        completion
            .get("completed_at")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0,
        "completed_at must be populated (wall-clock ms): {completion}"
    );
}

// ── Test B: SQLite store ────────────────────────────────────────────────────
//
// Same wire contract as Test A but against a per-harness SQLite file so the
// pg/sqlite portable column contract (TEXT / INTEGER, no JSONB operators)
// is exercised end-to-end. The no-DB-specific-features memory mandates
// parity across backends; this is the integration-level proof.

#[tokio::test]
async fn run_detail_carries_completion_summary_and_verification_sqlite() {
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    let marker = format!("f47pr2-sqlt-warn-{suffix}");
    let claim = format!("all checks passed cleanly {suffix}");

    let h = LiveHarness::setup_with_sqlite().await;
    let (run_id, _session_id, _tenant) = drive_run_to_completed(&h, &marker, &claim).await;

    let mut body = Value::Null;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let (status, b) = fetch_run(&h, &run_id).await;
        assert_eq!(status, 200);
        if b.get("completion").is_some() {
            body = b;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let completion = body
        .get("completion")
        .unwrap_or_else(|| panic!("F47 PR2 (sqlite): `completion` missing. body={body}"));
    assert_eq!(
        completion.get("summary").and_then(Value::as_str),
        Some(claim.as_str()),
        "sqlite: summary must survive TEXT roundtrip verbatim: {completion}"
    );
    let verification = completion
        .get("verification")
        .unwrap_or_else(|| panic!("sqlite: verification missing: {completion}"));
    let warnings = verification
        .get("warnings")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("sqlite: warnings array missing: {verification}"));
    assert!(
        warnings
            .iter()
            .filter_map(Value::as_str)
            .any(|w| w.contains(&marker)),
        "sqlite: marker must survive JSON-over-TEXT roundtrip: {verification}"
    );
    assert!(
        completion
            .get("completed_at")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0,
    );
}

// ── Test C: running run carries no completion ──────────────────────────────
//
// Inverse of A/B. A run that has NOT yet terminated (or that terminated on
// a non-Completed branch) MUST NOT surface a `completion` object. This
// guards against a regression where a partial projection (e.g. the event
// being appended on startup replay before the run reaches terminal state)
// fabricates a completion annotation on a non-completed run.

#[tokio::test]
async fn run_detail_completion_absent_for_running_run() {
    let h = LiveHarness::setup().await;

    let scope_suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let session_id = format!("sess_f47pr2run_{scope_suffix}");
    let run_id = format!("run_f47pr2run_{scope_suffix}");

    // Session + run (no orchestrate — the run stays Pending).
    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    tenant,
            "workspace_id": workspace,
            "project_id":   project,
            "session_id":   session_id,
        }))
        .send()
        .await
        .expect("session");
    assert_eq!(r.status().as_u16(), 201);

    let r = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    tenant,
            "workspace_id": workspace,
            "project_id":   project,
            "session_id":   session_id,
            "run_id":       run_id,
        }))
        .send()
        .await
        .expect("run");
    assert_eq!(r.status().as_u16(), 201);

    let (status, body) = fetch_run(&h, &run_id).await;
    assert_eq!(status, 200);
    // Absent field OR explicit null — both mean "no annotation." The
    // handler uses `skip_serializing_if = Option::is_none` so the field
    // should be absent, but accept either shape to keep the test robust
    // against a future response-shape policy change.
    let completion = body.get("completion");
    assert!(
        completion.is_none() || completion == Some(&Value::Null),
        "running run must NOT carry a completion annotation: {body}"
    );
}

// ── Test D: replay safety for terminal runs with no annotation ──────────────
//
// A terminal run whose event log contains no `RunCompletionAnnotated`
// event (e.g. a run that terminated before F47 PR2 shipped, or a run
// force-failed via operator intervention — `force_fail` does not emit the
// annotation event) must surface `completion: null` — no crash, no
// corrupted response shape. This is the "replay safety" contract the
// migration commits to: pre-F47-PR2 rows and non-completed terminations
// never get spurious completion annotations.

#[tokio::test]
async fn run_detail_completion_absent_when_no_annotation_event() {
    let h = LiveHarness::setup().await;

    let scope_suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let session_id = format!("sess_f47pr2na_{scope_suffix}");
    let run_id = format!("run_f47pr2na_{scope_suffix}");

    // Session + run.
    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    tenant,
            "workspace_id": workspace,
            "project_id":   project,
            "session_id":   session_id,
        }))
        .send()
        .await
        .expect("session");
    assert_eq!(r.status().as_u16(), 201);

    let r = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    tenant,
            "workspace_id": workspace,
            "project_id":   project,
            "session_id":   session_id,
            "run_id":       run_id,
        }))
        .send()
        .await
        .expect("run");
    assert_eq!(r.status().as_u16(), 201);

    // Force-fail via operator intervention. `force_fail` drives a direct
    // `RunStateChanged` + `OperatorIntervention` pair and — unlike
    // `force_complete` — does not require an active FF lease, so it
    // works on a still-Pending run. Neither intervention emits
    // `RunCompletionAnnotated`; this is the "legacy" / pre-F47-PR2
    // terminal-run shape the replay-safety contract covers.
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/intervene", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "action": "force_fail",
            "reason": "F47 PR2 replay-safety test — no annotation should land",
        }))
        .send()
        .await
        .expect("intervene");
    assert_eq!(
        r.status().as_u16(),
        200,
        "intervene body: {}",
        r.text().await.unwrap_or_default()
    );

    let (status, body) = fetch_run(&h, &run_id).await;
    assert_eq!(status, 200);
    assert_eq!(
        body.get("run")
            .and_then(|r| r.get("state"))
            .and_then(Value::as_str),
        Some("failed"),
        "intervene/force_fail must flip state to failed: {body}"
    );
    let completion = body.get("completion");
    assert!(
        completion.is_none() || completion == Some(&Value::Null),
        "terminal run with no annotation event must NOT surface \
         `completion` — that would corrupt the replay-safety contract. \
         body={body}"
    );
}
