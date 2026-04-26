//! F46 dogfood regression — tool_result from drained approved tool calls
//! must reach the NEXT DECIDE's user message so the LLM can see what
//! happened and decide the next step.
//!
//! **Bug (dogfood M1, 2026-04-26).** An operator approved a `bash: cargo
//! init` proposal. The orchestrator re-entered after approval, the F25
//! drain dispatched the bash successfully, `cargo init` wrote files to
//! disk. On the next DECIDE the LLM proposed the **same** `cargo init`
//! command again. Operator approved. Same thing. Four iterations, four
//! identical proposals — the LLM had no memory of the prior execution
//! because the drain path emitted a `StepSummary` of
//! `"drained approved: bash"` with no tool output attached. Pre-F46,
//! only the main (no-approval) path's `build_step_summary` embedded
//! `tool_result[<name>] ok: <preview>` lines into step history; the
//! drain path stripped the payload.
//!
//! Separately, a **rejected** proposal left no `StepSummary` at all,
//! so a persistent LLM that re-proposed a rejected command saw no
//! rejection reason either.
//!
//! **Fix.** `loop_runner::run_inner` now:
//!   1. Enriches drained approved-path summaries with the same
//!      `tool_result[<name>] ok|ERROR: <preview>` grammar as the main
//!      path, using [`truncate_for_summary`] for memory-bounding.
//!   2. Adds a `drain_rejected_pending` pass that emits one
//!      `StepSummary` per rejection carrying the operator-supplied
//!      reason, so the next DECIDE sees the rejection verbatim.
//!
//! Both enrichments flow through the same `build_user_message`
//! `step_history` section, so no changes to the decide-phase prompt
//! plumbing were needed.
//!
//! Test asserts the contract end-to-end over the real HTTP surface:
//!   1. LLM proposes approval-gated `bash`.
//!   2. Operator approves.
//!   3. Orchestrate re-entered; drain dispatches, bash runs.
//!   4. On the NEXT DECIDE the mock provider captures the raw request
//!      body. The user message MUST contain both the drain marker
//!      (`drained approved: bash`) and the `tool_result[bash] ok:`
//!      preview. Pre-F46 the body contained only `drained approved:
//!      bash` with no payload.

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

const MODEL_ID: &str = "openrouter/f46-feedback-model";

#[derive(Clone)]
struct MockState {
    hits: Arc<AtomicUsize>,
    /// Every received chat-completions body, in arrival order. Used by
    /// the test to assert what the LLM saw on the post-drain DECIDE.
    bodies: Arc<Mutex<Vec<Value>>>,
    marker_token: String,
}

async fn spawn_llm_mock(marker_token: String) -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<Value>>>) {
    let state = MockState {
        hits: Arc::new(AtomicUsize::new(0)),
        bodies: Arc::new(Mutex::new(Vec::new())),
        marker_token: marker_token.clone(),
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

        // Response strategy:
        //   hit 0 → propose an approval-gated bash that echoes a unique
        //           marker to stdout. Marker doubles as a needle the
        //           test searches for in the post-drain user message.
        //   hit 1+ → complete_run. Any further DECIDE calls get a
        //            complete_run too so the run terminates.
        let content_json = if n == 0 {
            json!([{
                "action_type": "invoke_tool",
                "description": "echo a marker token",
                "confidence": 0.99,
                "tool_name": "bash",
                "tool_args": {
                    // printf writes to stdout without a trailing newline
                    // so the captured output contains the marker
                    // verbatim. The harness-bash adapter surfaces
                    // stdout on the ToolResult value, which
                    // build_step_summary (main path) and now the drain
                    // path (F46 fix) render into
                    // `tool_result[bash] ok: <preview>`.
                    "command": format!("printf '%s' {:?}", state.marker_token),
                },
                "requires_approval": true
            }])
        } else {
            json!([{
                "action_type": "complete_run",
                "description": "marker echoed — done",
                "confidence": 0.99,
                "requires_approval": false
            }])
        };

        (
            StatusCode::OK,
            Json(json!({
                "id": format!("mock-f46-{n}"),
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
    (format!("http://{addr}"), hits, bodies)
}

/// Extract the concatenated user-message content across every
/// `messages[*]` entry with `role == "user"` in a chat-completions
/// request body. The drain summary is embedded into the step-history
/// section of whichever user message the decide-phase emits, so
/// concatenation is safe and keeps the assertion robust against
/// role-splitting changes in `build_user_message`.
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

#[tokio::test]
async fn drained_tool_result_reaches_next_decide_user_message() {
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    // Marker embeds the suffix so concurrent tests cannot collide on
    // the substring assertion, and so a spurious match from unrelated
    // log output is essentially impossible.
    let marker_token = format!("f46-marker-{suffix}");

    let h = LiveHarness::setup().await;
    let (mock_url, hits, bodies) = spawn_llm_mock(marker_token.clone()).await;

    let suffix2 = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_f46_{suffix2}");
    let session_id = format!("sess_f46_{suffix2}");
    let run_id = format!("run_f46_{suffix2}");

    // ── Credential + connection ─────────────────────────────────────────
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-f46-{suffix2}"),
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

    // ── Session + run ──────────────────────────────────────────────────
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

    // ── 1st orchestrate: proposal → 202 waiting_approval ───────────────
    let first = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "echo a marker token via an approval-gated bash command",
            "max_iterations": 3,
            "approval_timeout_ms": 30_000u64,
        }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .expect("first orchestrate");
    assert_eq!(first.status().as_u16(), 202, "first orchestrate must 202");

    // ── Find + approve the proposal ─────────────────────────────────────
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
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
            .expect("list approvals");
        if r.status().as_u16() == 200 {
            let body: Value = r.json().await.expect("list json");
            let items = body
                .get("items")
                .and_then(|v| v.as_array())
                .cloned()
                .or_else(|| body.as_array().cloned())
                .unwrap_or_default();
            if let Some(first_item) = items.first() {
                if first_item.get("tool_name").and_then(|v| v.as_str()) == Some("bash") {
                    if let Some(cid) = first_item.get("call_id").and_then(|v| v.as_str()) {
                        call_id = Some(cid.to_owned());
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let call_id = call_id.expect("pending approval did not appear");

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
        .expect("approve");
    assert_eq!(r.status().as_u16(), 200);

    // ── 2nd orchestrate: drain dispatches bash, next DECIDE sees output ─
    let second = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "echo a marker token via an approval-gated bash command",
            // 3 iterations gives: (1) drain + DECIDE that should see
            // tool_result + call complete_run, (2) fallback slack, (3)
            // hard ceiling. Pre-F46 the LLM would re-propose bash here
            // (no memory of prior exec) and the test would trip its
            // substring assertion instead of the iteration cap.
            "max_iterations": 3,
            "approval_timeout_ms": 30_000u64,
        }))
        .timeout(Duration::from_secs(90))
        .send()
        .await
        .expect("second orchestrate");
    let st = second.status().as_u16();
    let body_txt = second.text().await.unwrap_or_default();
    assert_eq!(
        st, 200,
        "second orchestrate must 200 post-approval; body={body_txt}"
    );

    // ── CORE F46 assertion ─────────────────────────────────────────────
    // After the drain ran, the orchestrator MUST have made at least one
    // more DECIDE call (post-approval, post-drain). That DECIDE's user
    // message must contain BOTH the drain marker (F25 wire trace) and a
    // substring of the tool output (F46 fix). We use the marker_token
    // because it is unique, deterministic, and small enough to survive
    // truncate_for_summary (400-char cap).
    let captured = bodies.lock().expect("bodies mutex").clone();
    assert!(
        hits.load(Ordering::SeqCst) >= 2,
        "F46: post-approval DECIDE never ran (hits={}). body={body_txt}",
        hits.load(Ordering::SeqCst),
    );
    // The relevant body is the LAST one (the post-drain DECIDE). Scan
    // every post-first body in case the orchestrator issues extra
    // plan-mode or nudge calls along the way — the fix is confirmed as
    // long as SOME post-first DECIDE sees the tool_result grammar.
    let found = captured.iter().skip(1).any(|b| {
        let text = user_text(b);
        text.contains("drained approved: bash") && text.contains("tool_result[bash] ok:")
    });
    assert!(
        found,
        "F46 regression: no post-drain DECIDE carried the tool_result into \
         the user message. This is the wire bug — the LLM would re-propose \
         the same command because it has no memory of the prior execution. \
         Captured bodies ({} total): user texts = {:#?}",
        captured.len(),
        captured
            .iter()
            .map(user_text)
            .collect::<Vec<_>>(),
    );
    // Strong form: the marker token should flow through to the user
    // message verbatim (it's the bash stdout). If the drain enrichment
    // ever regresses from "embed preview" back to "bare header", this
    // assertion trips before the grammar one.
    let marker_found = captured
        .iter()
        .skip(1)
        .any(|b| user_text(b).contains(&marker_token));
    assert!(
        marker_found,
        "F46 regression: marker token {marker_token:?} missing from every \
         post-drain DECIDE body — drain summary shed the stdout payload. \
         Bodies: {:#?}",
        captured.iter().map(user_text).collect::<Vec<_>>()
    );
}

/// F46 rejection variant. An operator-rejected proposal must leave a
/// `tool_result[<name>] REJECTED: <reason>` line in the next DECIDE's
/// user message so the LLM sees WHY the operator rejected and does not
/// re-propose the same call.
#[tokio::test]
async fn rejected_proposal_reason_reaches_next_decide_user_message() {
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    let rejection_reason = format!("cairn-f46-reject-reason-{suffix}");

    let h = LiveHarness::setup().await;
    let (mock_url, hits, bodies) = spawn_llm_mock(format!("ignored-{suffix}")).await;

    let suffix2 = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_f46r_{suffix2}");
    let session_id = format!("sess_f46r_{suffix2}");
    let run_id = format!("run_f46r_{suffix2}");

    // Standard cred + connection + defaults + session + run.
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-f46r-{suffix2}"),
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
        .expect("connection");
    assert_eq!(r.status().as_u16(), 201);

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

    // 1st orchestrate → 202 waiting_approval
    let first = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "propose a bash the operator will reject",
            "max_iterations": 3,
            "approval_timeout_ms": 30_000u64,
        }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .expect("first orchestrate");
    assert_eq!(first.status().as_u16(), 202);

    // Locate + REJECT the proposal with a unique reason.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
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
            .expect("list approvals");
        if r.status().as_u16() == 200 {
            let body: Value = r.json().await.unwrap_or(Value::Null);
            let items = body
                .get("items")
                .and_then(|v| v.as_array())
                .cloned()
                .or_else(|| body.as_array().cloned())
                .unwrap_or_default();
            if let Some(first_item) = items.first() {
                if let Some(cid) = first_item.get("call_id").and_then(|v| v.as_str()) {
                    call_id = Some(cid.to_owned());
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let call_id = call_id.expect("pending approval did not appear");

    let r = h
        .client()
        .post(format!(
            "{}/v1/tool-call-approvals/{}/reject",
            h.base_url, call_id
        ))
        .header("X-Cairn-Tenant", &tenant)
        .header("X-Cairn-Workspace", &workspace)
        .header("X-Cairn-Project", &project)
        .bearer_auth(&h.admin_token)
        .json(&json!({ "reason": rejection_reason }))
        .send()
        .await
        .expect("reject");
    assert_eq!(
        r.status().as_u16(),
        200,
        "reject: {}",
        r.text().await.unwrap_or_default()
    );

    // 2nd orchestrate → rejection drain surfaces the reason
    let second = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": "propose a bash the operator will reject",
            "max_iterations": 3,
            "approval_timeout_ms": 30_000u64,
        }))
        .timeout(Duration::from_secs(90))
        .send()
        .await
        .expect("second orchestrate");
    let st = second.status().as_u16();
    let body_txt = second.text().await.unwrap_or_default();
    assert_eq!(st, 200, "second orchestrate must 200; body={body_txt}");

    let captured = bodies.lock().expect("bodies mutex").clone();
    assert!(
        hits.load(Ordering::SeqCst) >= 2,
        "F46: post-rejection DECIDE never ran (hits={}). body={body_txt}",
        hits.load(Ordering::SeqCst),
    );
    let found = captured
        .iter()
        .skip(1)
        .any(|b| user_text(b).contains(&rejection_reason));
    assert!(
        found,
        "F46 regression: rejection reason {rejection_reason:?} missing from every \
         post-rejection DECIDE body — rejection drain did not surface the reason. \
         Bodies: {:#?}",
        captured.iter().map(user_text).collect::<Vec<_>>(),
    );
}
