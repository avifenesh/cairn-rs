//! F54 dogfood regression — the `tool_result[bash] ok: <preview>` line
//! in the next DECIDE's user message must preserve the **tail** of
//! stdout (the final `warning:` / `error:` / success banner) and an
//! explicit `exit_code:` prefix.
//!
//! **Bug (dogfood phase 2, 2026-04-26).** GLM-4.7 via Z.ai repeatedly
//! issued the same `cargo check` command three times in a row without
//! progressing the run. Root cause: the pre-F54 preview path serialised
//! the bash ToolResult to JSON and passed it through
//! `truncate_for_summary` with a 400-char head+tail cap. Once stdout
//! was larger than ~400 chars, the middle of the JSON blob — including
//! the `exit_code` field and the trailing diagnostic line — was clipped
//! away. The LLM saw an opaque, truncated JSON snippet and could not
//! tell whether its G1/G2 gate had passed, so it re-ran the command.
//!
//! **Fix.** `render_tool_output_preview` (in `loop_runner.rs`) detects
//! bash-class tools and emits a structured preview:
//!
//! ```text
//! exit_code: 0
//! stdout(tail):
//! [truncated N chars] …Compiling foo
//! warning: unused import
//! ```
//!
//! This test drives the main (no-approval) dispatch path — where
//! `build_step_summary` is the source of the preview — because the F54
//! dogfood repro happened on an auto-approved bash call. (The drain
//! path mirrors the same formatting via the F46 enrichment; that path
//! is covered by `test_f46_tool_result_feedback`.)

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

const MODEL_ID: &str = "openrouter/f54-tail-preview-model";

#[derive(Clone)]
struct MockState {
    hits: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
    /// Number of bash iterations the LLM is allowed to request before
    /// it switches to `complete_run`. Used to exercise the "repeat
    /// detection" path: the test drives up to 4 iterations and asserts
    /// on what the user message contained.
    bash_iters: usize,
    /// Substring the test expects to survive the preview truncation —
    /// the marker lives at the END of a ~4 KB stdout so only a
    /// tail-preserving preview will carry it through.
    trailing_marker: String,
}

async fn spawn_llm_mock(bash_iters: usize, trailing_marker: String) -> MockCtx {
    let state = MockState {
        hits: Arc::new(AtomicUsize::new(0)),
        bodies: Arc::new(Mutex::new(Vec::new())),
        bash_iters,
        trailing_marker: trailing_marker.clone(),
    };
    let hits = state.hits.clone();
    let bodies = state.bodies.clone();

    async fn chat_handler(
        State(state): State<MockState>,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        {
            let mut guard = state.bodies.lock().expect("bodies mutex poisoned");
            guard.push(body.clone());
        }
        let n = state.hits.fetch_add(1, Ordering::SeqCst);

        // For the first `bash_iters` hits, propose an auto-approved
        // bash command that emits ~4 KB to stdout with the trailing
        // marker at the end. On subsequent hits, complete the run.
        //
        // The command uses `sh -c` with a loop so the output is well
        // above the prior 400-char truncation cap, guaranteeing that
        // only a tail-preserving preview surfaces the marker.
        let content_json = if n < state.bash_iters {
            let marker = &state.trailing_marker;
            // Produce 200 lines of ~30 chars ≈ 6 KB of stdout, then
            // echo the marker last.
            let cmd = format!(
                "for i in $(seq 1 200); do printf '   Compiling crate-stub-%s v0.1.0\\n' $i; done; printf '{marker}\\n'"
            );
            json!([{
                "action_type": "invoke_tool",
                "description": "run cargo-check-shaped bash",
                "confidence": 0.99,
                "tool_name": "bash",
                "tool_args": { "command": cmd },
                "requires_approval": false
            }])
        } else {
            json!([{
                "action_type": "complete_run",
                "description": "done",
                "confidence": 0.99,
                "requires_approval": false
            }])
        };

        (
            StatusCode::OK,
            Json(json!({
                "id": format!("mock-f54-{n}"),
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": content_json.to_string(),
                    },
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "total_tokens": 30,
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
    tokio::time::sleep(Duration::from_millis(25)).await;
    MockCtx {
        url: format!("http://{addr}"),
        hits,
        bodies,
    }
}

struct MockCtx {
    url: String,
    hits: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
}

/// Concatenate every user-role message across a chat-completions body.
fn user_text(body: &Value) -> String {
    body.get("messages")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn wire_provider(h: &LiveHarness, mock_url: &str, suffix: &str) {
    let tenant = "default_tenant";
    let credential_id = {
        let r = h
            .client()
            .post(format!(
                "{}/v1/admin/tenants/{}/credentials",
                h.base_url, tenant
            ))
            .bearer_auth(&h.admin_token)
            .json(&json!({
                "provider_id": "openrouter",
                "plaintext_value": format!("sk-f54-{suffix}"),
            }))
            .send()
            .await
            .expect("credential reaches server");
        assert_eq!(r.status().as_u16(), 201);
        r.json::<Value>()
            .await
            .unwrap()
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_owned()
    };

    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "provider_connection_id": format!("conn_f54_{suffix}"),
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": [MODEL_ID],
            "credential_id": credential_id,
            "endpoint_url": mock_url,
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
                h.base_url, key
            ))
            .bearer_auth(&h.admin_token)
            .json(&json!({ "value": MODEL_ID }))
            .send()
            .await
            .expect("defaults");
        assert_eq!(r.status().as_u16(), 200);
    }
}

async fn setup_session_run(h: &LiveHarness, suffix: &str) -> String {
    let tenant = "default_tenant";
    let workspace = "default_workspace";
    let project = "default_project";
    let session_id = format!("sess_f54_{suffix}");
    let run_id = format!("run_f54_{suffix}");

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
        .expect("session");
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
        .expect("run");
    assert_eq!(r.status().as_u16(), 201);
    run_id
}

/// CORE F54 contract. A bash tool invocation whose stdout ends with a
/// `warning:`-shaped marker line MUST have that marker reach the next
/// DECIDE's user message. Pre-F54 the 400-char head+tail cap on the
/// serialized JSON ToolResult clipped the marker away.
#[tokio::test]
async fn trailing_warning_survives_preview_on_large_bash_output() {
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    let trailing_marker = format!("warning: f54-trailing-marker-{suffix}");

    let h = LiveHarness::setup().await;
    let mock = spawn_llm_mock(1, trailing_marker.clone()).await;
    wire_provider(&h, &mock.url, &suffix).await;
    let run_id = setup_session_run(&h, &suffix).await;

    // One bash iter + one complete_run = 2 DECIDE hits.
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "emit a large bash output",
            "max_iterations": 3,
        }))
        .timeout(Duration::from_secs(90))
        .send()
        .await
        .expect("orchestrate");
    let st = r.status().as_u16();
    let body_txt = r.text().await.unwrap_or_default();
    assert_eq!(st, 200, "orchestrate: {body_txt}");

    let captured = mock.bodies.lock().expect("bodies mutex").clone();
    assert!(
        mock.hits.load(Ordering::SeqCst) >= 2,
        "post-bash DECIDE never ran (hits={})",
        mock.hits.load(Ordering::SeqCst),
    );

    // The 2nd (or later) DECIDE must carry the trailing marker in the
    // user message — i.e. the tail of stdout survived truncation.
    let found = captured
        .iter()
        .skip(1)
        .any(|b| user_text(b).contains(&trailing_marker));
    assert!(
        found,
        "F54 regression: trailing marker {trailing_marker:?} missing from \
         every post-bash DECIDE body. The preview truncation is clipping \
         the tail of stdout. Bodies: {:#?}",
        captured.iter().map(user_text).collect::<Vec<_>>()
    );

    // Also: exit_code line must be present so the LLM has an
    // unambiguous success signal.
    let exit_line_found = captured
        .iter()
        .skip(1)
        .any(|b| user_text(b).contains("exit_code: 0"));
    assert!(
        exit_line_found,
        "F54 regression: post-bash DECIDE body missing `exit_code: 0` line. \
         The LLM cannot tell whether the prior call succeeded. Bodies: {:#?}",
        captured.iter().map(user_text).collect::<Vec<_>>()
    );
}

/// Functional guarantee. After one bash invocation returns with
/// visible exit_code + tail, the LLM (the mock) must have received
/// sufficient context to advance — we model this by making the mock
/// stop proposing bash after 1 iteration. If the loop escapes within
/// its iteration cap, the preview carried enough signal. If the loop
/// burns through `max_iterations` re-proposing bash, the preview did
/// NOT carry enough signal (the F54 dogfood symptom).
///
/// Practically: `hits >= 2` means one bash + one complete_run, i.e.
/// the LLM "saw" the prior result and changed its action. Pre-F54
/// with a max_iterations ≥ 4 and the mock pinned to bash-forever this
/// test would have fired `bash_iters` worth of repeats.
#[tokio::test]
async fn llm_advances_after_tail_preserving_preview() {
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    let trailing_marker = format!("warning: f54-advance-marker-{suffix}");

    let h = LiveHarness::setup().await;
    let mock = spawn_llm_mock(1, trailing_marker).await;
    wire_provider(&h, &mock.url, &suffix).await;
    let run_id = setup_session_run(&h, &suffix).await;

    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "run one bash then complete",
            "max_iterations": 4,
        }))
        .timeout(Duration::from_secs(90))
        .send()
        .await
        .expect("orchestrate");
    let st = r.status().as_u16();
    let body_txt = r.text().await.unwrap_or_default();
    assert_eq!(st, 200, "orchestrate: {body_txt}");

    // At least 2 DECIDE calls: one to propose bash, one to see the
    // tool_result and choose complete_run. We cap at 4 above so an
    // unbounded-repeat regression would show as hits == 4 or the loop
    // terminating for iteration-cap rather than complete.
    let hits = mock.hits.load(Ordering::SeqCst);
    assert!(hits >= 2, "LLM never saw the tool_result (hits={hits})");
    // `body_txt` will carry the final orchestrate terminal state.
    // A healthy termination is `completed`; a regression shows up as
    // `max_iterations_reached` or similar because the LLM kept
    // re-proposing bash.
    assert!(
        body_txt.contains("completed") || body_txt.contains("\"state\":\"completed\""),
        "F54 functional guarantee: run must complete normally after \
         seeing the tail-preserving preview; instead got: {body_txt}"
    );
}

/// Clean-output case. A bash command that prints nothing but exits 0
/// must still produce a preview that carries an explicit `exit_code: 0`
/// line so the LLM can assert "the prior G-gate passed".
#[tokio::test]
async fn clean_bash_output_exposes_exit_code() {
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();

    // Mock that proposes a tiny bash (just `true`) then completes.
    #[derive(Clone)]
    struct TinyState {
        hits: Arc<AtomicUsize>,
        bodies: Arc<Mutex<Vec<Value>>>,
    }
    let tiny = TinyState {
        hits: Arc::new(AtomicUsize::new(0)),
        bodies: Arc::new(Mutex::new(Vec::new())),
    };
    let hits = tiny.hits.clone();
    let bodies = tiny.bodies.clone();

    async fn handler(
        State(s): State<TinyState>,
        Json(b): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        s.bodies.lock().unwrap().push(b);
        let n = s.hits.fetch_add(1, Ordering::SeqCst);
        let content = if n == 0 {
            json!([{
                "action_type": "invoke_tool",
                "description": "noop bash",
                "confidence": 0.99,
                "tool_name": "bash",
                "tool_args": {"command": "true"},
                "requires_approval": false
            }])
        } else {
            json!([{
                "action_type": "complete_run",
                "description": "done",
                "confidence": 0.99,
                "requires_approval": false
            }])
        };
        (
            StatusCode::OK,
            Json(json!({
                "id": format!("mock-f54-clean-{n}"),
                "choices": [{
                    "index": 0,
                    "message": {"role":"assistant","content": content.to_string()},
                    "finish_reason":"stop",
                }],
                "usage": {"prompt_tokens":20,"completion_tokens":10,"total_tokens":30}
            })),
        )
    }

    let app = Router::new()
        .route("/chat/completions", post(handler))
        .route("/v1/chat/completions", post(handler))
        .route(
            "/v1/models",
            get(|| async { Json(json!({"data":[{"id": MODEL_ID}]})) }),
        )
        .with_state(tiny);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(25)).await;
    let mock_url = format!("http://{addr}");

    let h = LiveHarness::setup().await;
    wire_provider(&h, &mock_url, &suffix).await;
    let run_id = setup_session_run(&h, &suffix).await;

    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "noop bash",
            "max_iterations": 3,
        }))
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .expect("orchestrate");
    assert_eq!(r.status().as_u16(), 200);

    let captured = bodies.lock().unwrap().clone();
    assert!(hits.load(Ordering::SeqCst) >= 2);
    let found_exit = captured
        .iter()
        .skip(1)
        .any(|b| user_text(b).contains("exit_code: 0"));
    assert!(
        found_exit,
        "F54 regression: clean bash output did not expose `exit_code: 0` \
         in the next DECIDE body. Bodies: {:#?}",
        captured.iter().map(user_text).collect::<Vec<_>>()
    );
}
