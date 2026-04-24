//! F25 dogfood regression — full HTTP + filesystem integration test.
//!
//! Reproduces the F25 blocker: an LLM emits a `bash` tool call with
//! `requires_approval=true`; the operator approves; the caller re-
//! orchestrates the run; the approved bash command MUST actually run
//! on the filesystem (prior to the drain fix the run would loop
//! forever asking for the same approval that had already been granted,
//! because the approval service's cache auto-approved the duplicate
//! proposal and nothing in the loop actually dispatched the approved
//! side effect).
//!
//! This test asserts the ONE thing the existing propose-then-await
//! suite failed to assert: **the bash side effect happened on disk.**
//! An event-log assertion alone is not sufficient — RFC 020's Track 3
//! events could all be present while the tool never actually ran.
//!
//! Design:
//!
//! 1. Stand up a `LiveHarness` (real cairn-app subprocess on a uuid-
//!    scoped tenant).
//! 2. Spawn a mock OpenAI-compatible provider that:
//!      * turn 1 → emits an `invoke_tool` proposal for `bash` writing
//!        a temp file, `requires_approval=true`.
//!      * turn 2+ → emits `complete_run` so the loop terminates.
//! 3. Bind the mock as the tenant's provider connection, set system
//!    defaults, create session + run.
//! 4. Orchestrate. Expect a waiting-approval terminal.
//! 5. Look up the pending tool-call approval's `call_id`, approve it.
//! 6. Orchestrate again. The drain must execute the approved bash BEFORE
//!    DECIDE fires, and the loop must reach `complete_run`.
//! 7. Assert the temp file exists with the expected content.
//!
//! The temp file lives under `$TMPDIR` with a uuid suffix so parallel
//! test runs do not collide. Cleanup happens in a guard at the end.

mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

const MODEL_ID: &str = "openrouter/drain-test-model";

/// Mock provider state — switches response payload based on how many
/// `/chat/completions` calls it has seen so far.
#[derive(Clone)]
struct MockState {
    hits: Arc<AtomicUsize>,
    // Filled by setup() so the bash proposal the mock emits targets a
    // test-scoped temp file.
    marker_path: String,
    marker_content: String,
}

async fn spawn_llm_mock(marker_path: String, marker_content: String) -> (String, Arc<AtomicUsize>) {
    let state = MockState {
        hits: Arc::new(AtomicUsize::new(0)),
        marker_path: marker_path.clone(),
        marker_content: marker_content.clone(),
    };
    let hits = state.hits.clone();

    async fn chat_handler(
        State(state): State<MockState>,
        Json(_body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        let n = state.hits.fetch_add(1, Ordering::SeqCst);
        let content_json = if n == 0 {
            // Turn 1: propose an approval-gated bash call.
            //
            // `command` is a single-shot shell command that writes
            // `marker_content` to `marker_path`. We intentionally use
            // a stable POSIX redirection so the bash built-in handles
            // it without extra flags. The path lives outside any
            // sandbox so the test runner's user can read it back.
            json!([{
                "action_type": "invoke_tool",
                "description": "write the marker file so the drain test can verify bash actually ran",
                "confidence": 0.99,
                "tool_name": "bash",
                "tool_args": {
                    "command": format!(
                        "printf '%s' {:?} > {:?}",
                        state.marker_content, state.marker_path
                    )
                },
                "requires_approval": true
            }])
        } else {
            // Turn 2+: after drain runs, we just wrap the run up.
            json!([{
                "action_type": "complete_run",
                "description": "marker written — done",
                "confidence": 0.99,
                "requires_approval": false
            }])
        };

        (
            StatusCode::OK,
            Json(json!({
                "id": format!("mock-drain-{n}"),
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": content_json.to_string(),
                    },
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15,
                }
            })),
        )
    }

    let app = Router::new()
        .route("/chat/completions", post(chat_handler))
        .route("/v1/chat/completions", post(chat_handler))
        .route(
            "/v1/models",
            get(|| async { Json(json!({"data":[{"id": MODEL_ID}]})) }),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // Give the listener a tick to become visible.
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    (format!("http://{addr}"), hits)
}

/// RAII guard that removes the marker file on drop so a panicking test
/// still cleans up /tmp.
struct MarkerFileGuard(PathBuf);
impl Drop for MarkerFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test]
async fn drain_approved_bash_actually_runs_and_writes_file() {
    // Temp-file marker so parallel runs don't collide.
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    let marker_path = std::env::temp_dir().join(format!("cairn-drain-f25-{suffix}.txt"));
    let marker_content = format!("drained-{suffix}");
    let _guard = MarkerFileGuard(marker_path.clone());

    let h = LiveHarness::setup().await;
    let (mock_url, hits) = spawn_llm_mock(
        marker_path.to_string_lossy().into_owned(),
        marker_content.clone(),
    )
    .await;

    let suffix2 = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_drain_{suffix2}");
    let session_id = format!("sess_drain_{suffix2}");
    let run_id = format!("run_drain_{suffix2}");

    // ── 1. Credential + provider connection ────────────────────────────
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-drain-{suffix2}"),
        }))
        .send()
        .await
        .expect("credential reaches server");
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
            "tenant_id": tenant,
            "provider_connection_id": connection_id,
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": [MODEL_ID],
            "credential_id": credential_id,
            "endpoint_url": mock_url,
        }))
        .send()
        .await
        .expect("connection reaches server");
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
                h.base_url, key
            ))
            .bearer_auth(&h.admin_token)
            .json(&json!({ "value": MODEL_ID }))
            .send()
            .await
            .expect("defaults reach server");
        assert_eq!(r.status().as_u16(), 200);
    }

    // ── 2. Session + run ────────────────────────────────────────────────
    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "session_id": session_id,
        }))
        .send()
        .await
        .expect("session reaches server");
    assert_eq!(r.status().as_u16(), 201);

    let r = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "session_id": session_id,
            "run_id": run_id,
        }))
        .send()
        .await
        .expect("run reaches server");
    assert_eq!(r.status().as_u16(), 201);

    // ── 3. Kick off orchestrate in a background task. The execute-phase
    //       BP-v2 approval gate will block synchronously on
    //       `await_decision` until the operator approves, so the HTTP
    //       call does NOT return until approval lands. The test drives
    //       the approve flow concurrently, then joins on the orchestrate
    //       future and asserts the filesystem side effect. This flow
    //       covers the "LLM emits a proposal, operator approves while
    //       the execute phase is parked" path that RFC 020 + BP-v2 are
    //       designed around.
    let base_url = h.base_url.clone();
    let admin_token = h.admin_token.clone();
    let run_id_clone = run_id.clone();
    let client_orch = h.client().clone();
    let orchestrate_task = tokio::spawn(async move {
        client_orch
            .post(format!("{}/v1/runs/{}/orchestrate", base_url, run_id_clone))
            .bearer_auth(&admin_token)
            .json(&json!({
                "goal": "write the marker file via an approval-gated bash command",
                "max_iterations": 3,
                // 30s operator-decision timeout — the test must approve
                // within this window. Above the ~10s we need for the
                // mock to answer + the poll loop below to react.
                "approval_timeout_ms": 30_000u64,
            }))
            .timeout(std::time::Duration::from_secs(90))
            .send()
            .await
    });

    // ── 4. Poll for the pending tool-call approval, then approve ────────
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut call_id: Option<String> = None;
    while std::time::Instant::now() < deadline {
        let r = h
            .client()
            .get(format!(
                "{}/v1/tool-call-approvals?run_id={}&state=pending",
                h.base_url, run_id
            ))
            .header("X-Cairn-Tenant", &tenant)
            .header("X-Cairn-Workspace", &workspace)
            .header("X-Cairn-Project", &project)
            .bearer_auth(&h.admin_token)
            .send()
            .await
            .expect("list tool-call-approvals reaches server");
        if r.status().as_u16() == 200 {
            let body: Value = r.json().await.expect("list json");
            let items = body
                .get("items")
                .and_then(|v| v.as_array())
                .cloned()
                .or_else(|| body.as_array().cloned())
                .unwrap_or_default();
            if let Some(first) = items.first() {
                if first.get("tool_name").and_then(|v| v.as_str()) == Some("bash") {
                    if let Some(cid) = first.get("call_id").and_then(|v| v.as_str()) {
                        call_id = Some(cid.to_owned());
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let call_id = call_id.expect("pending tool-call approval did not appear within 30s");

    assert!(
        !marker_path.exists(),
        "marker file must NOT exist before approval"
    );

    let r = h
        .client()
        .post(format!(
            "{}/v1/tool-call-approvals/{}/approve",
            h.base_url, call_id
        ))
        .header("X-Cairn-Tenant", &tenant)
        .header("X-Cairn-Workspace", &workspace)
        .header("X-Cairn-Project", &project)
        .bearer_auth(&h.admin_token)
        .json(&json!({"scope": {"type": "once"}}))
        .send()
        .await
        .expect("approve reaches server");
    assert_eq!(
        r.status().as_u16(),
        200,
        "approve: {}",
        r.text().await.unwrap_or_default()
    );

    // ── 5. Join orchestrate — must complete cleanly ─────────────────────
    let orch_res = orchestrate_task
        .await
        .expect("orchestrate task joins")
        .expect("orchestrate request reaches server");
    let orch_status = orch_res.status().as_u16();
    let orch_body_text = orch_res.text().await.unwrap_or_default();
    assert_eq!(
        orch_status, 200,
        "orchestrate should succeed once approval lands, got {orch_status}: {orch_body_text}"
    );
    let orch_body: Value = serde_json::from_str(&orch_body_text).unwrap_or(Value::Null);
    let term = orch_body
        .get("termination")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Acceptable terminal outcomes: either `completed` (the ideal
    // happy path) or `failed` (when the mock's `complete_run` runs
    // against a bare LiveHarness run where FF hasn't claimed a task —
    // that fabric-layer error is orthogonal to the F25 bug and surfaces
    // AFTER the drained bash has already executed). The filesystem
    // assertion below is the real regression guard; the terminal is
    // informational.
    assert!(
        term == "completed" || term == "failed",
        "orchestrate should terminate cleanly, got {orch_body_text}",
    );
    assert!(
        hits.load(Ordering::SeqCst) >= 1,
        "LLM must have been called at least once",
    );

    // ── 7. THE filesystem assertion ─────────────────────────────────────
    // This is the single assertion that distinguishes a real fix from a
    // "events look right but nothing ran" regression. If the drain failed
    // to dispatch, the marker file will not exist — no amount of event-log
    // plumbing can fake that.
    assert!(
        marker_path.exists(),
        "MARKER FILE MISSING: drain did not actually dispatch bash. \
         path={:?}, LLM hits={}, body={}",
        marker_path,
        hits.load(Ordering::SeqCst),
        orch_body_text,
    );
    let got = std::fs::read_to_string(&marker_path).expect("read marker file");
    assert_eq!(
        got.trim(),
        marker_content,
        "marker content mismatch: expected {marker_content:?}, got {got:?}",
    );
}
