//! F47 PR1 regression — the `orchestrate_finished` SSE event must carry a
//! `completion_verification` sidecar that surfaces warning lines present in
//! tool_result frames, independent of the LLM's free-text `summary`.
//!
//! **Bug (dogfood M1, 2026-04-26).** The LLM produced a Rust crate that
//! emitted `warning: unused imports: Constraint, Direction, Layout,
//! text::Line` in a stored bash tool_result, then claimed in its
//! `complete_run` summary that "cargo check must pass with no warnings ✓".
//! Operators had no independent signal that the summary lied.
//!
//! **Fix (PR1).** The loop runner accumulates every `ActionResult` across a
//! run. On `LoopSignal::Done`, a pure extractor scans the accumulated
//! tool_results for warning / error lines and per-bash-command exit codes,
//! producing a `CompletionVerification`. The orchestrator's SSE emitter
//! attaches the sidecar to the `orchestrate_finished` frame under
//! `completion_verification`. This test asserts the wire-level contract:
//! drive a real HTTP run whose bash tool emits a marker warning, consume
//! the SSE stream, assert the Finished frame contains the marker in
//! `completion_verification.warnings`.
//!
//! PR2 will add persistence (event + projection + REST). PR3 will add UI
//! rendering. This PR stays above the event/store layer — no new persisted
//! fields.

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
use futures::StreamExt;
use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

const MODEL_ID: &str = "openrouter/f47-verification-model";

#[derive(Clone)]
struct MockState {
    hits: Arc<AtomicUsize>,
    marker: String,
}

/// Two-round scripted provider:
///   round 0 → invoke_tool bash (no approval), stdout carries `warning: …`
///   round 1+ → complete_run with an overclaim ("no warnings") so the test
///              proves the verification sidecar contradicts the summary.
async fn spawn_mock(marker: String) -> (String, Arc<AtomicUsize>) {
    let state = MockState {
        hits: Arc::new(AtomicUsize::new(0)),
        marker: marker.clone(),
    };
    let hits = state.hits.clone();

    async fn chat_handler(
        State(state): State<MockState>,
        Json(_body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        let n = state.hits.fetch_add(1, Ordering::SeqCst);
        let content = if n == 0 {
            // Single `printf` emits the warning marker to stdout. The
            // cairn bash adapter surfaces stdout on the tool_result,
            // which the F47 extractor scans line-by-line. The marker
            // starts with `warning:` so it lands in the warnings bucket.
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
            // Overclaim: the summary says "no warnings" while the
            // tool_result above contained one. The SSE verification
            // sidecar MUST surface the warning regardless.
            json!([{
                "action_type":       "complete_run",
                "description":       "done — no warnings",
                "confidence":        0.99,
                "requires_approval": false,
            }])
        };
        (
            StatusCode::OK,
            Json(json!({
                "id":      format!("mock-f47-{n}"),
                "choices": [{
                    "index":  0,
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

/// Parse a chunk of SSE wire text into a vector of `(event_name, data)`
/// pairs. Handles frames separated by blank lines and the `event:` /
/// `data:` field layout defined by the EventSource spec.
fn parse_sse_frames(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for frame in raw.split("\n\n") {
        let mut event: Option<String> = None;
        let mut data: Vec<String> = Vec::new();
        for line in frame.lines() {
            if let Some(v) = line.strip_prefix("event:") {
                event = Some(v.trim().to_owned());
            } else if let Some(v) = line.strip_prefix("data:") {
                data.push(v.trim_start().to_owned());
            }
        }
        if let Some(name) = event {
            out.push((name, data.join("\n")));
        }
    }
    out
}

/// F47 PR1 core assertion. The SSE `orchestrate_finished` frame for a
/// Completed run must carry a `completion_verification` object whose
/// `warnings` array surfaces warning lines from the tool_result stream.
#[tokio::test]
async fn orchestrate_finished_carries_completion_verification_warnings() {
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    // Embed the suffix so concurrent test runs cannot collide on the
    // substring match and so unrelated log chatter cannot satisfy the
    // assertion by accident.
    let marker = format!("f47-unused-imports-{suffix}");

    let h = LiveHarness::setup().await;
    let (mock_url, _hits) = spawn_mock(marker.clone()).await;

    let scope_suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_f47_{scope_suffix}");
    let session_id = format!("sess_f47_{scope_suffix}");
    let run_id = format!("run_f47_{scope_suffix}");

    // ── Credential + connection + defaults ─────────────────────────────
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id":     "openrouter",
            "plaintext_value": format!("sk-f47-{scope_suffix}"),
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

    // ── Session + run ──────────────────────────────────────────────────
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

    // ── Drive the orchestrate loop to Completed ────────────────────────
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
        "run must terminate Completed for verification to be emitted: {orch_body}"
    );

    // ── Consume SSE — Last-Event-Id: 0 replays every buffered frame, so
    //    subscribing AFTER the run completes still gives us the Finished
    //    frame from the in-memory 10k-entry replay buffer.
    let sse = h
        .client()
        .get(format!("{}/v1/streams/runtime", h.base_url))
        .bearer_auth(&h.admin_token)
        .header("last-event-id", "0")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .expect("sse");
    assert_eq!(sse.status().as_u16(), 200);
    assert!(sse
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .contains("text/event-stream"));

    // Drain up to ~1 MB or 3 seconds from the never-closing stream;
    // `orchestrate_finished` lands in the first handful of frames after
    // replay, so 1 MB is orders of magnitude over-provisioned.
    let mut buf = String::new();
    let mut stream = sse.bytes_stream();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline && buf.len() < 1_000_000 {
        match tokio::time::timeout(Duration::from_millis(250), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                // Fast exit once we have the frame we need to avoid
                // waiting for the keep-alive ping.
                if buf.contains("orchestrate_finished") {
                    break;
                }
            }
            Ok(Some(Err(_))) => break,
            Ok(None) => break,
            Err(_) => continue, // timeout on this poll; try again
        }
    }

    let frames = parse_sse_frames(&buf);
    let finished = frames
        .iter()
        .filter_map(|(_, data)| serde_json::from_str::<Value>(data).ok())
        .find(|v| {
            v.get("event").and_then(Value::as_str) == Some("orchestrate_finished")
                && v.get("termination").and_then(Value::as_str) == Some("completed")
        })
        .unwrap_or_else(|| {
            panic!(
                "F47 regression: never saw `orchestrate_finished / completed` in SSE. \
                 Raw buffer ({} bytes): {}",
                buf.len(),
                &buf[..buf.len().min(2_000)],
            )
        });

    // CORE F47 ASSERTION: the Finished frame carries a verification
    // object with the marker warning surfaced.
    let verification = finished.get("completion_verification").unwrap_or_else(|| {
        panic!("F47 regression: `completion_verification` missing from finished frame: {finished}")
    });
    let warnings = verification
        .get("warnings")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("F47 regression: `warnings` array missing: {verification}"));
    let found = warnings
        .iter()
        .filter_map(Value::as_str)
        .any(|w| w.contains(&marker));
    assert!(
        found,
        "F47 regression: marker {marker:?} did not surface in `warnings`. \
         This is the wire-level bug the PR closes: the LLM claimed no \
         warnings while the bash tool_result contained one, and without \
         the sidecar operators had no signal. verification={verification}",
    );
    assert_eq!(
        verification
            .get("extractor_version")
            .and_then(Value::as_u64),
        Some(1),
        "extractor_version stamp missing/wrong — downstream consumers rely \
         on this to detect shape drift: {verification}"
    );
    assert!(
        verification
            .get("tool_results_scanned")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            >= 1,
        "tool_results_scanned must be >= 1 for a run that invoked bash: \
         {verification}"
    );
    // commands[] should carry the bash invocation with no exit-code
    // fabrication. The harness bash adapter may or may not surface
    // exit_code structurally; the field presence/absence is asserted via
    // the type, not the value.
    let commands = verification
        .get("commands")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("commands array missing: {verification}"));
    assert!(
        commands
            .iter()
            .any(|c| c.get("tool_name").and_then(Value::as_str) == Some("bash")),
        "bash invocation not recorded in commands[]: {commands:#?}"
    );
}
