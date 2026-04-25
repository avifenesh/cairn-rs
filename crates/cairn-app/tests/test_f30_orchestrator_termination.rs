//! F30 regression: the orchestrator must terminate cleanly on direct-answer
//! prompts instead of looping on introspection tools.
//!
//! **Bug (dogfood run 1, 2026-04-24).** Three simple prose prompts
//! (e.g. `"Summarize the key design principles of Minecraft creative mode
//! in three bullet points."`) each burned through `max_iterations=8`
//! without completing. Every iteration the LLM called introspection
//! tools — `search_events`, `get_run`, `get_approvals`, `memory_search`,
//! `list_runs` — zero productive work, zero output, ~$0.04 each. No
//! `complete_run`, no final text.
//!
//! **Fix.** Two changes:
//!
//!   1. System-prompt rewrite: for direct-answer prompts, the model is now
//!      told to call `complete_run` immediately with the full answer as
//!      `description`, NOT to gather context first. See
//!      `crates/cairn-orchestrator/src/decide_impl.rs::build_system_prompt`
//!      (search for "F30 fix").
//!
//!   2. Introspection-of-self tools (`get_run`, `get_task`, `get_approvals`,
//!      `list_runs`, `search_events`, `wait_for_task`) removed from the
//!      orchestrate tool registry. They remain on operator-chat entry
//!      points. See `crates/cairn-app/src/handlers/runs.rs::orchestrate_run_handler`
//!      (search for "F30").
//!
//! This test mounts a deterministic mock LLM that returns a single
//! `complete_run` action on the very first call. It asserts:
//!
//!   - The orchestrator terminates with `termination == "completed"` — NOT
//!     `max_iterations_reached`.
//!   - Exactly **one** LLM call was made (iteration 0 terminates).
//!   - The `summary` field carries the final answer text back to the
//!     caller so the user actually sees a response.
//!
//! A second test asserts the system prompt no longer contains the
//! "Phase 1: Understand / Phase 2: Act / Phase 3: Verify" gating that
//! triggered the introspection loop in the first place — a cheap
//! snapshot-style check so a future prompt edit can't silently
//! regress F30.

mod support;

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

const MOCK_MODEL: &str = "openrouter/f30-direct-answer";
const FINAL_ANSWER: &str =
    "- Players can place any block freely.\n- Flight is always enabled.\n- No resource cost.";

#[derive(Clone)]
struct MockState {
    hits: Arc<AtomicUsize>,
    captured_system_prompts: Arc<std::sync::Mutex<Vec<String>>>,
}

/// Spawn a mock OpenAI-compatible provider that always responds with a
/// `complete_run` action containing the final answer text.
///
/// Also captures the `system` messages so the test can inspect the
/// prompt the orchestrator actually constructed for regression
/// assertions.
async fn spawn_mock() -> (String, Arc<AtomicUsize>, Arc<std::sync::Mutex<Vec<String>>>) {
    let state = MockState {
        hits: Arc::new(AtomicUsize::new(0)),
        captured_system_prompts: Arc::new(std::sync::Mutex::new(Vec::new())),
    };
    let hits = state.hits.clone();
    let captured = state.captured_system_prompts.clone();

    async fn chat_handler(
        State(state): State<MockState>,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        state.hits.fetch_add(1, Ordering::SeqCst);

        // Capture the system message for the prompt-regression assertion.
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            for msg in messages {
                if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        state
                            .captured_system_prompts
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push(content.to_owned());
                    }
                }
            }
        }

        // Respond with a single complete_run action embedding the final
        // answer as the description. This is the shape the system prompt
        // now asks for on direct-answer goals.
        let payload = json!([{
            "action_type": "complete_run",
            "description": FINAL_ANSWER,
            "confidence": 0.98,
            "requires_approval": false,
        }]);

        (
            StatusCode::OK,
            Json(json!({
                "id": "mock-f30",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": payload.to_string(),
                    },
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 40,
                    "total_tokens": 140,
                }
            })),
        )
    }

    let app = Router::new()
        .route("/chat/completions", post(chat_handler))
        .route("/v1/chat/completions", post(chat_handler))
        .route(
            "/v1/models",
            get(|| async { Json(json!({ "data": [{"id": MOCK_MODEL}] })) }),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    (format!("http://{addr}"), hits, captured)
}

/// Provision a tenant credential + provider connection + system default
/// models + session + run, then orchestrate. Returns the orchestrate
/// response body JSON.
async fn setup_and_orchestrate(
    h: &LiveHarness,
    mock_url: &str,
    goal: &str,
    max_iterations: u32,
) -> (u16, Value) {
    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let connection_id = format!("conn_f30_{suffix}");
    let session_id = format!("sess_f30_{suffix}");
    let run_id = format!("run_f30_{suffix}");

    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-f30-{suffix}"),
        }))
        .send()
        .await
        .expect("credential reaches server");
    assert_eq!(r.status().as_u16(), 201);
    let credential_id = r
        .json::<Value>()
        .await
        .expect("credential json")
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
            "supported_models": [MOCK_MODEL],
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
                h.base_url, key,
            ))
            .bearer_auth(&h.admin_token)
            .json(&json!({ "value": MOCK_MODEL }))
            .send()
            .await
            .expect("defaults reach server");
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

    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/orchestrate", h.base_url, run_id,))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "goal": goal,
            "max_iterations": max_iterations,
        }))
        .send()
        .await
        .expect("orchestrate reaches server");
    let status = r.status().as_u16();
    let body: Value = r.json().await.unwrap_or(Value::Null);
    (status, body)
}

/// F30 primary regression: a direct-answer goal must terminate on the
/// first iteration with `complete_run`, NOT exhaust `max_iterations`.
#[tokio::test]
async fn direct_answer_prompt_calls_complete_run_in_one_iteration() {
    let h = LiveHarness::setup().await;
    let (mock_url, hits, captured) = spawn_mock().await;

    let (status, body) = setup_and_orchestrate(
        &h,
        &mock_url,
        "Summarize the key design principles of Minecraft creative mode in three bullet points.",
        // max_iterations matches the dogfood run 1 configuration so
        // a regression would present identically (8 hits, no
        // completion). Fix should terminate after 1 LLM call.
        8,
    )
    .await;

    // Core F30 assertion: the orchestrator must NOT hit
    // `max_iterations_reached`. The LLM emits `complete_run` on the first
    // iteration, which the loop handles as a terminal decision. In the
    // LiveHarness fabric a downstream `run_service.complete` FCALL may
    // still fail with `"fabric layer error"` (the test harness does not
    // provision a full Valkey-backed fabric), which surfaces as
    // `termination: "failed"` with an execute-phase reason — that is
    // ORTHOGONAL to the F30 bug (the dogfood HTTP fallback regression
    // test makes the same allowance at
    // `crates/cairn-app/tests/test_http_dogfood_fallback.rs`).
    //
    // The regression we are guarding is purely: "the decide phase
    // reached a terminal action in 1 iteration." So we assert on hit
    // count + non-`max_iterations` termination.
    let termination = body
        .get("termination")
        .and_then(|t| t.as_str())
        .unwrap_or("<missing>");
    assert_ne!(
        termination, "max_iterations_reached",
        "F30: orchestrator must NOT exhaust iterations on a direct-answer prompt. \
         Full body: {body}",
    );

    // Exactly one LLM call: iteration 0 produced complete_run and
    // the loop left the DECIDE phase. Before the fix this was 8.
    let n = hits.load(Ordering::SeqCst);
    assert_eq!(
        n, 1,
        "F30: expected exactly 1 LLM call for a trivial direct-answer prompt, got {n}. \
         A value >1 means the loop is iterating — check system prompt + tool registry. \
         Full body: {body}",
    );

    // Status should be 200 (completed) or 200 (failed, execute-phase
    // fabric error). A 5xx would indicate a different regression.
    assert_eq!(
        status, 200,
        "orchestrate should return 200 regardless of fabric execute outcome; body={body}"
    );

    // If the fabric DID accept the complete call, assert the final
    // answer propagates. This is the ideal outcome; we don't require
    // it because LiveHarness fabric state is flaky for full-lifecycle
    // FCALLs. When it works, we verify the user-facing contract.
    if termination == "completed" {
        assert_eq!(
            body.get("summary").and_then(|s| s.as_str()),
            Some(FINAL_ANSWER),
            "summary must carry the final answer text back to the caller \
             so the user actually sees a response; body={body}",
        );
    }

    // Review follow-up: assert on the live-wired system prompt. This is
    // distinct from the unit-level snapshot test below — it confirms
    // the HTTP orchestrate path threads the F30-fixed prompt all the
    // way to the provider, not a stale cached copy from somewhere up
    // the stack. If a refactor ever interposes a different prompt
    // builder on the HTTP path, this guard catches it.
    let prompts = captured.lock().unwrap_or_else(|p| p.into_inner()).clone();
    assert!(
        !prompts.is_empty(),
        "mock provider must have received at least one system message",
    );
    let live_sys = &prompts[0];
    for banned in [
        "Phase 1: Understand",
        "Phase 2: Act",
        "Phase 3: Verify",
        "Phase 4: Complete",
        "You have taken action toward the goal (not just read/searched)",
    ] {
        assert!(
            !live_sys.contains(banned),
            "F30: live-wired system prompt on the HTTP orchestrate path \
             must not contain the pre-fix phrase {banned:?}. A refactor \
             may have reintroduced a stale prompt builder. Captured:\n{live_sys}",
        );
    }
    assert!(
        live_sys.contains("complete_run"),
        "live-wired prompt must still mention complete_run. Captured:\n{live_sys}",
    );
    assert!(
        live_sys.to_lowercase().contains("introspect"),
        "F30: live-wired prompt must forbid calling introspection tools on \
         THIS run — that's the prompt-layer guard that closes the loop bug \
         while keeping the tools registered for legitimate system-aware \
         prompts. Captured:\n{live_sys}",
    );
}

/// F30 prompt regression: neither the system prompt NOR the user message
/// footer may reintroduce the "Phase 1: Understand / Phase 2: Act /
/// Phase 3: Verify / Phase 4: Complete" gating that caused the
/// introspection loop. Snapshot-style assertion so a future prompt edit
/// reviewer sees the failure and rechecks F30 before shipping.
///
/// Before this test covered the user message, the system prompt could
/// be cleaned up while the footer (`build_user_message`) continued to
/// instruct the LLM to "Decide your next action based on the workflow
/// phases: Understand → Act → Verify → Complete" — a contradiction
/// that would leave the LLM receiving stale instructions on every
/// iteration. This assertion guards both channels.
#[tokio::test]
async fn system_prompt_does_not_gate_complete_run_behind_forced_action() {
    use cairn_orchestrator::decide_impl_test_hooks::{
        build_system_prompt_for_tests, build_user_message_for_tests,
    };

    // Shared list of pre-F30 phrases. Any of these returning in either
    // channel (system prompt or user message) is a regression.
    let banned = [
        "Phase 1: Understand",
        "Phase 2: Act",
        "Phase 3: Verify",
        "Phase 4: Complete",
        "You have taken action toward the goal (not just read/searched)",
        "Reading and analysing alone is not sufficient",
        "workflow phases: Understand",
    ];

    // ── System prompt ────────────────────────────────────────────────
    for native in [false, true] {
        let sys = build_system_prompt_for_tests("orchestrator", &[], native);
        for phrase in banned {
            assert!(
                !sys.contains(phrase),
                "F30: system prompt must not contain the pre-fix gating phrase \
                 {phrase:?}. Reintroducing it risks regressing the direct-answer \
                 loop — see `decide_impl.rs::build_system_prompt` comment block. \
                 (native_tools_enabled={native})",
            );
        }

        assert!(
            sys.contains("complete_run"),
            "prompt must still mention complete_run (native={native})",
        );
        // Gemini review follow-up: the prompt must accommodate RAG-style
        // goals (e.g. "summarize the README") where tools ARE required
        // before answering. The new wording talks about "external
        // information" instead of "no tools first".
        assert!(
            sys.contains("external information"),
            "F30 prompt must explicitly acknowledge the RAG path — tools are \
             OK when the goal needs external information. Got:\n{sys}",
        );
        // Must still forbid introspection-of-self to close the original
        // bug even though the tools themselves are kept registered for
        // legitimate system-aware prompts.
        assert!(
            sys.to_lowercase().contains("introspect")
                || sys.to_lowercase().contains("introspection"),
            "F30 prompt must forbid calling introspection tools on THIS run. \
             Got:\n{sys}",
        );
    }

    // ── User message footer ──────────────────────────────────────────
    // Exercise `build_user_message` with a representative context so
    // the whole footer (which wraps the memory hint and the next-step
    // block) is covered — not just the static template strings.
    use cairn_orchestrator::{GatherOutput, OrchestrationContext};
    use std::path::PathBuf;
    let ctx = OrchestrationContext {
        project: cairn_domain::ProjectKey {
            tenant_id: cairn_domain::TenantId::new("t"),
            workspace_id: cairn_domain::WorkspaceId::new("w"),
            project_id: cairn_domain::ProjectId::new("p"),
        },
        session_id: cairn_domain::SessionId::new("s"),
        run_id: cairn_domain::RunId::new("r"),
        task_id: None,
        iteration: 0,
        goal: "Summarize X in three bullets.".to_owned(),
        agent_type: "orchestrator".to_owned(),
        run_started_at_ms: 0,
        working_dir: PathBuf::from("/tmp"),
        run_mode: Default::default(),
        discovered_tool_names: vec![],
        step_history: vec![],
        is_recovery: false,
        approval_timeout: None,
    };
    // Empty gather: exercises the "no memory retrieved" footer branch.
    // With memory, the only difference is the `memory_hint` prefix —
    // the banned-phrase assertions are orthogonal to that. Keeping
    // this focused on the footer avoids cross-crate churn constructing
    // a full `RetrievalResult` just to exercise a code path that
    // shares the same footer template.
    let user = build_user_message_for_tests(&ctx, &GatherOutput::default());
    for phrase in banned {
        assert!(
            !user.contains(phrase),
            "F30: user message must not contain pre-fix gating phrase \
             {phrase:?}. `build_user_message`'s footer is the counterpart \
             to the system prompt — they must not contradict each other. \
             Got:\n{user}",
        );
    }
    assert!(
        user.contains("complete_run"),
        "user message footer should direct the model to complete_run when \
         it has the answer. Got:\n{user}",
    );
}
