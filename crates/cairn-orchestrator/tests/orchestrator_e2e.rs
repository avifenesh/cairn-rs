//! End-to-end orchestrator integration tests.
//!
//! # Test organisation
//!
//! ## Mock tests (always run, no API key required)
//!
//! These tests exercise the full GATHER → DECIDE → EXECUTE plumbing using a
//! `StubDecidePhase` that returns a deterministic `CompleteRun` proposal.
//! They prove the loop wiring, event emission, and termination logic are
//! correct without any network calls.
//!
//! ## Live tests (`#[ignore]`)
//!
//! These tests call the real OpenRouter API with the model
//! `openrouter/auto`.  They are gated by `#[ignore]` so they
//! **only run when explicitly requested**:
//!
//! ```sh
//! OPENROUTER_API_KEY=sk-or-… \
//!   cargo test -p cairn-orchestrator --test orchestrator_e2e -- --ignored
//! ```
//!
//! OpenRouter compatibility note: OpenRouter exposes an OpenAI-compatible
//! `/v1/chat/completions` endpoint. `OpenAiCompat` works without any
//! modification — just point `base_url` at `https://openrouter.ai/api/v1`
//! and supply the bearer token.

use std::path::PathBuf;
use std::sync::Arc;

use cairn_domain::{ActionProposal, ActionType, ProjectKey, RunId, SessionId};
use cairn_orchestrator::{
    DecideOutput, GatherOutput, LoopConfig, LoopTermination, OrchestrationContext,
    OrchestratorError, OrchestratorLoop, RuntimeExecutePhase, StandardGatherPhase,
};
use cairn_runtime::{
    services::{
        ApprovalServiceImpl, CheckpointServiceImpl, MailboxServiceImpl, RunServiceImpl,
        TaskServiceImpl, ToolInvocationServiceImpl,
    },
    InMemoryServices, SessionService,
};
use cairn_store::EventLog;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("e2e_tenant", "e2e_ws", "e2e_proj")
}
fn session_id() -> SessionId {
    SessionId::new("e2e_session")
}
fn run_id() -> RunId {
    RunId::new("e2e_run")
}

async fn setup_run() -> Arc<InMemoryServices> {
    let svc = Arc::new(InMemoryServices::new());
    svc.sessions.create(&project(), session_id()).await.unwrap();
    svc.runs
        .start(&project(), &session_id(), run_id(), None)
        .await
        .unwrap();
    svc
}

fn build_execute_phase(svc: &Arc<InMemoryServices>) -> RuntimeExecutePhase {
    build_execute_phase_with_registry(svc, None)
}

fn build_execute_phase_with_registry(
    svc: &Arc<InMemoryServices>,
    registry: Option<Arc<cairn_tools::BuiltinToolRegistry>>,
) -> RuntimeExecutePhase {
    let store = svc.store.clone();
    let mut builder = RuntimeExecutePhase::builder()
        .run_service(Arc::new(RunServiceImpl::new(store.clone())))
        .task_service(Arc::new(TaskServiceImpl::new(store.clone())))
        .approval_service(Arc::new(ApprovalServiceImpl::new(store.clone())))
        .checkpoint_service(Arc::new(CheckpointServiceImpl::new(store.clone())))
        .mailbox_service(Arc::new(MailboxServiceImpl::new(store.clone())))
        .tool_invocation_service(Arc::new(ToolInvocationServiceImpl::new(store)))
        .checkpoint_every_n_tool_calls(1);
    if let Some(reg) = registry {
        builder = builder.tool_registry(reg);
    }
    builder.build()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn base_ctx() -> OrchestrationContext {
    OrchestrationContext {
        project: project(),
        session_id: session_id(),
        run_id: run_id(),
        task_id: None,
        iteration: 0,
        goal: "List 3 facts about Rust programming language".to_owned(),
        agent_type: "test_agent".to_owned(),
        run_started_at_ms: now_ms(),
        working_dir: PathBuf::from("."),
        run_mode: cairn_domain::decisions::RunMode::Direct,
        discovered_tool_names: vec![],
    }
}

// ── Stub DecidePhase ──────────────────────────────────────────────────────────

/// A deterministic decide phase that immediately proposes CompleteRun.
/// Used for plumbing tests that don't require a real LLM.
struct StubDecidePhase {
    response_text: String,
}

#[async_trait::async_trait]
impl cairn_orchestrator::DecidePhase for StubDecidePhase {
    async fn decide(
        &self,
        _ctx: &OrchestrationContext,
        _gather: &GatherOutput,
    ) -> Result<DecideOutput, OrchestratorError> {
        Ok(DecideOutput {
            raw_response: self.response_text.clone(),
            proposals: vec![ActionProposal::complete_run("stub: task done", 0.99)],
            calibrated_confidence: 0.99,
            requires_approval: false,
            model_id: "stub".to_owned(),
            latency_ms: 0,
            input_tokens: None,
            output_tokens: None,
        })
    }
}

// ── Mock / plumbing tests ─────────────────────────────────────────────────────

/// Full loop completes in one step when the decide phase returns CompleteRun.
#[tokio::test]
async fn loop_completes_when_decide_returns_complete_run() {
    let svc = setup_run().await;
    let store = svc.store.clone();

    // Transition the run to Running so RunService::complete is valid.
    use cairn_domain::lifecycle::RunState;
    use cairn_domain::{EventEnvelope, EventId, EventSource, RunStateChanged, StateTransition};
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_running_e2e"),
            EventSource::Runtime,
            cairn_domain::RuntimeEvent::RunStateChanged(RunStateChanged {
                project: project(),
                run_id: run_id(),
                transition: StateTransition {
                    from: Some(RunState::Pending),
                    to: RunState::Running,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
        )])
        .await
        .unwrap();

    let gather = StandardGatherPhase::builder(store.clone()).build();
    let decide = StubDecidePhase {
        response_text: "complete".into(),
    };
    let execute = build_execute_phase(&svc);

    let termination = OrchestratorLoop::new(gather, decide, execute, LoopConfig::default())
        .run(base_ctx())
        .await
        .unwrap();

    assert!(
        matches!(termination, LoopTermination::Completed { .. }),
        "expected Completed, got {termination:?}"
    );
}

/// At least one event is emitted to the store during a full loop run.
#[tokio::test]
async fn loop_emits_events_to_store() {
    let svc = setup_run().await;
    let store = svc.store.clone();

    // Transition to Running.
    use cairn_domain::lifecycle::RunState;
    use cairn_domain::{EventEnvelope, EventId, EventSource, RunStateChanged, StateTransition};
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_run_e2e_events"),
            EventSource::Runtime,
            cairn_domain::RuntimeEvent::RunStateChanged(RunStateChanged {
                project: project(),
                run_id: run_id(),
                transition: StateTransition {
                    from: Some(RunState::Pending),
                    to: RunState::Running,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
        )])
        .await
        .unwrap();

    let events_before = store.head_position().await.unwrap();

    let gather = StandardGatherPhase::builder(store.clone()).build();
    let decide = StubDecidePhase {
        response_text: "complete".into(),
    };
    let execute = build_execute_phase(&svc);

    OrchestratorLoop::new(gather, decide, execute, LoopConfig::default())
        .run(base_ctx())
        .await
        .unwrap();

    let events_after = store.head_position().await.unwrap();
    assert!(
        events_after > events_before,
        "loop must emit at least one new event to the store"
    );
}

/// Max-iterations terminates the loop rather than running forever.
#[tokio::test]
async fn loop_terminates_at_max_iterations() {
    use cairn_orchestrator::decide::DecidePhase;

    struct ContinueForeverPhase;

    #[async_trait::async_trait]
    impl DecidePhase for ContinueForeverPhase {
        async fn decide(
            &self,
            _ctx: &OrchestrationContext,
            _g: &GatherOutput,
        ) -> Result<DecideOutput, OrchestratorError> {
            Ok(DecideOutput {
                raw_response: "{}".into(),
                proposals: vec![ActionProposal {
                    action_type: ActionType::CreateMemory,
                    description: "store a fact".into(),
                    confidence: 0.5,
                    tool_name: None,
                    tool_args: Some(serde_json::json!({ "content": "fact" })),
                    requires_approval: false,
                }],
                calibrated_confidence: 0.5,
                requires_approval: false,
                model_id: "stub".into(),
                latency_ms: 0,
                input_tokens: None,
                output_tokens: None,
            })
        }
    }

    let svc = setup_run().await;
    let store = svc.store.clone();
    let gather = StandardGatherPhase::builder(store.clone()).build();
    let decide = ContinueForeverPhase;
    let execute = build_execute_phase(&svc);
    let config = LoopConfig {
        max_iterations: 3,
        ..LoopConfig::default()
    };

    let termination = OrchestratorLoop::new(gather, decide, execute, config)
        .run(base_ctx())
        .await
        .unwrap();

    assert!(
        matches!(termination, LoopTermination::MaxIterationsReached),
        "expected MaxIterationsReached, got {termination:?}"
    );
}

/// Approval gate suspends the loop and returns WaitingApproval.
#[tokio::test]
async fn loop_suspends_on_requires_approval() {
    use cairn_orchestrator::decide::DecidePhase;

    struct ApprovalRequiredPhase;

    #[async_trait::async_trait]
    impl DecidePhase for ApprovalRequiredPhase {
        async fn decide(
            &self,
            _ctx: &OrchestrationContext,
            _g: &GatherOutput,
        ) -> Result<DecideOutput, OrchestratorError> {
            Ok(DecideOutput {
                raw_response: "{}".into(),
                proposals: vec![ActionProposal::escalate("need approval", 0.5)],
                calibrated_confidence: 0.5,
                requires_approval: true,
                model_id: "stub".into(),
                latency_ms: 0,
                input_tokens: None,
                output_tokens: None,
            })
        }
    }

    let svc = setup_run().await;
    let store = svc.store.clone();

    // Transition to Running so approval request succeeds.
    use cairn_domain::lifecycle::RunState;
    use cairn_domain::{EventEnvelope, EventId, EventSource, RunStateChanged, StateTransition};
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_appr_e2e"),
            EventSource::Runtime,
            cairn_domain::RuntimeEvent::RunStateChanged(RunStateChanged {
                project: project(),
                run_id: run_id(),
                transition: StateTransition {
                    from: Some(RunState::Pending),
                    to: RunState::Running,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
        )])
        .await
        .unwrap();

    let gather = StandardGatherPhase::builder(store.clone()).build();
    let decide = ApprovalRequiredPhase;
    let execute = build_execute_phase(&svc);

    let termination = OrchestratorLoop::new(gather, decide, execute, LoopConfig::default())
        .run(base_ctx())
        .await
        .unwrap();

    assert!(
        matches!(termination, LoopTermination::WaitingApproval { .. }),
        "expected WaitingApproval, got {termination:?}"
    );
}

// ── Live tests (require OPENROUTER_API_KEY) ───────────────────────────────────

/// Live integration test: full GATHER → DECIDE → EXECUTE loop with
/// `openrouter/auto` via OpenRouter.
///
/// Run with:
/// ```sh
/// OPENROUTER_API_KEY=sk-or-…  \
///   cargo test -p cairn-orchestrator --test orchestrator_e2e \
///     live_openrouter_loop_completes -- --ignored --nocapture
/// ```
///
/// The test passes when the loop terminates (either `Completed` or
/// `MaxIterations`) and at least one event was emitted to the store,
/// proving the full plumbing works end-to-end with a real LLM.
#[tokio::test]
#[ignore = "requires OPENROUTER_API_KEY env var — run with --ignored"]
async fn live_openrouter_loop_completes() {
    let api_key =
        std::env::var("OPENROUTER_API_KEY").expect("set OPENROUTER_API_KEY to run live tests");

    use cairn_providers::wire::openai_compat::{OpenAiCompat, ProviderConfig};

    let brain_provider = Arc::new(OpenAiCompat::new(
        ProviderConfig::OPENROUTER,
        api_key,
        Some("https://openrouter.ai/api/v1".to_owned()),
        Some("openrouter/auto".to_owned()),
        None,
        None,
        None,
    ));

    // Model: openrouter/auto — 262K context, zero cost.
    let model_id = "openrouter/auto".to_owned();

    let svc = setup_run().await;
    let store = svc.store.clone();

    // Transition to Running.
    use cairn_domain::lifecycle::RunState;
    use cairn_domain::{EventEnvelope, EventId, EventSource, RunStateChanged, StateTransition};
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_live_or_running"),
            EventSource::Runtime,
            cairn_domain::RuntimeEvent::RunStateChanged(RunStateChanged {
                project: project(),
                run_id: run_id(),
                transition: StateTransition {
                    from: Some(RunState::Pending),
                    to: RunState::Running,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
        )])
        .await
        .unwrap();

    let events_before = store.head_position().await.unwrap();

    // Register only memory_search so the model has one valid read tool.
    // When memory_search returns empty results (no ingested docs), the model
    // should complete_run with its own knowledge rather than trying other tools.
    // We intentionally exclude web_fetch to avoid the model attempting HTTP calls
    // and then escalating when they return stub responses.
    let tool_registry = {
        use cairn_tools::builtins::{BuiltinToolRegistry, MemorySearchTool, MemoryStoreTool};
        Arc::new(
            BuiltinToolRegistry::new()
                .register(Arc::new(MemorySearchTool::new()))
                .register(Arc::new(MemoryStoreTool::new())),
        )
    };

    let gather = StandardGatherPhase::builder(store.clone()).build();
    let decide = cairn_orchestrator::LlmDecidePhase::new(brain_provider, model_id.clone())
        .with_tools(tool_registry.clone());
    // Pass the same registry to the execute phase so tool dispatch succeeds.
    let execute = build_execute_phase_with_registry(&svc, Some(tool_registry));

    let config = LoopConfig {
        max_iterations: 4,  // give the model a few attempts to complete
        timeout_ms: 90_000, // 90-second wall-clock timeout
        ..LoopConfig::default()
    };

    let termination = OrchestratorLoop::new(gather, decide, execute, config)
        .run(base_ctx())
        .await
        .unwrap();

    println!("termination: {termination:?}");

    // Primary goal: Completed (LLM used tools + wrote summary).
    // Acceptable fallbacks: MaxIterationsReached (tried but ran out of turns),
    //                       WaitingApproval/WaitingSubagent (valid LLM decisions).
    // Failure modes: Failed (infrastructure error), TimedOut (network stall).
    assert!(
        !matches!(termination, LoopTermination::TimedOut),
        "loop must not time out — got {termination:?}"
    );
    // Prefer Completed; log a warning for other outcomes.
    if matches!(termination, LoopTermination::Completed { .. }) {
        println!("SUCCESS: loop completed — LLM answered the goal directly");
    } else if let LoopTermination::Failed { reason } = &termination {
        // Unknown tool = model still making up names. Fail the test.
        panic!("FAIL: loop failed — {reason}");
    } else {
        println!("PARTIAL: loop terminated with {termination:?} (not ideal but acceptable)");
    }
    println!("loop terminated: {termination:?}");

    let events_after = store.head_position().await.unwrap();
    assert!(
        events_after > events_before,
        "loop must emit at least one new event to the store"
    );
}

/// Sanity-check that OpenRouter's /v1/models endpoint is reachable
/// and returns a model list.
#[tokio::test]
#[ignore = "requires OPENROUTER_API_KEY env var — run with --ignored"]
async fn live_openrouter_models_reachable() {
    let api_key =
        std::env::var("OPENROUTER_API_KEY").expect("set OPENROUTER_API_KEY to run live tests");

    let client = reqwest::Client::new();
    let resp = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .expect("request to openrouter.ai/api/v1/models failed");

    assert!(
        resp.status().is_success(),
        "expected 200, got {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let models = body["data"].as_array().unwrap();
    assert!(!models.is_empty(), "expected at least one model");

    let free_model = models
        .iter()
        .any(|m| m["id"].as_str().unwrap_or("").contains("free"));
    println!("OpenRouter has free models: {free_model}");
    println!("Total models available: {}", models.len());
}
