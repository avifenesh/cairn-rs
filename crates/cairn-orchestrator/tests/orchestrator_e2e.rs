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
//! `qwen/qwen3-coder:free`.  They are gated by `#[ignore]` so they
//! **only run when explicitly requested**:
//!
//! ```sh
//! OPENROUTER_API_KEY=sk-or-… \
//!   cargo test -p cairn-orchestrator --test orchestrator_e2e -- --ignored
//! ```
//!
//! OpenRouter compatibility note: OpenRouter exposes an OpenAI-compatible
//! `/v1/chat/completions` endpoint.  `OpenAiCompatProvider` works without
//! any modification — just point `base_url` at `https://openrouter.ai/api/v1`
//! and supply the bearer token.

use std::sync::Arc;

use cairn_domain::{
    ActionProposal, ActionType, ProjectKey, RunId, SessionId,
};
use cairn_orchestrator::{
    DecideOutput, ExecuteOutcome, GatherOutput, LoopConfig, LoopSignal,
    LoopTermination, OrchestratorError, OrchestratorLoop, OrchestrationContext,
    RuntimeExecutePhase, StandardGatherPhase,
};
use cairn_runtime::{
    InMemoryServices, RunService, SessionService,
    services::{
        ApprovalServiceImpl, CheckpointServiceImpl, MailboxServiceImpl,
        RunServiceImpl, TaskServiceImpl, ToolInvocationServiceImpl,
    },
};
use cairn_store::EventLog;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey { ProjectKey::new("e2e_tenant", "e2e_ws", "e2e_proj") }
fn session_id() -> SessionId { SessionId::new("e2e_session") }
fn run_id() -> RunId { RunId::new("e2e_run") }

async fn setup_run() -> Arc<InMemoryServices> {
    let svc = Arc::new(InMemoryServices::new());
    svc.sessions.create(&project(), session_id()).await.unwrap();
    svc.runs.start(&project(), &session_id(), run_id(), None).await.unwrap();
    svc
}

fn build_execute_phase(svc: &Arc<InMemoryServices>) -> RuntimeExecutePhase {
    let store = svc.store.clone();
    RuntimeExecutePhase::builder()
        .run_service(Arc::new(RunServiceImpl::new(store.clone())))
        .task_service(Arc::new(TaskServiceImpl::new(store.clone())))
        .approval_service(Arc::new(ApprovalServiceImpl::new(store.clone())))
        .checkpoint_service(Arc::new(CheckpointServiceImpl::new(store.clone())))
        .mailbox_service(Arc::new(MailboxServiceImpl::new(store.clone())))
        .tool_invocation_service(Arc::new(ToolInvocationServiceImpl::new(store)))
        .checkpoint_every_n_tool_calls(1)
        .build()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn base_ctx() -> OrchestrationContext {
    OrchestrationContext {
        project:           project(),
        session_id:        session_id(),
        run_id:            run_id(),
        task_id:           None,
        iteration:         0,
        goal:              "List 3 facts about Rust programming language".to_owned(),
        agent_type:        "test_agent".to_owned(),
        run_started_at_ms: now_ms(),
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
            raw_response:          self.response_text.clone(),
            proposals:             vec![ActionProposal::complete_run(
                "stub: task done", 0.99,
            )],
            calibrated_confidence: 0.99,
            requires_approval:     false,
            model_id:              "stub".to_owned(),
            latency_ms:            0,
        })
    }
}

// ── Mock / plumbing tests ─────────────────────────────────────────────────────

/// Full loop completes in one step when the decide phase returns CompleteRun.
#[tokio::test]
async fn loop_completes_when_decide_returns_complete_run() {
    let svc   = setup_run().await;
    let store = svc.store.clone();

    // Transition the run to Running so RunService::complete is valid.
    use cairn_domain::{EventEnvelope, EventId, EventSource, RunStateChanged, StateTransition};
    use cairn_domain::lifecycle::RunState;
    store.append(&[EventEnvelope::for_runtime_event(
        EventId::new("evt_running_e2e"),
        EventSource::Runtime,
        cairn_domain::RuntimeEvent::RunStateChanged(RunStateChanged {
            project:         project(),
            run_id:          run_id(),
            transition:      StateTransition { from: Some(RunState::Pending), to: RunState::Running },
            failure_class:   None,
            pause_reason:    None,
            resume_trigger:  None,
        }),
    )]).await.unwrap();

    let gather  = StandardGatherPhase::builder(store.clone()).build();
    let decide  = StubDecidePhase { response_text: "complete".into() };
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
    let svc   = setup_run().await;
    let store = svc.store.clone();

    // Transition to Running.
    use cairn_domain::{EventEnvelope, EventId, EventSource, RunStateChanged, StateTransition};
    use cairn_domain::lifecycle::RunState;
    store.append(&[EventEnvelope::for_runtime_event(
        EventId::new("evt_run_e2e_events"),
        EventSource::Runtime,
        cairn_domain::RuntimeEvent::RunStateChanged(RunStateChanged {
            project:        project(),
            run_id:         run_id(),
            transition:     StateTransition { from: Some(RunState::Pending), to: RunState::Running },
            failure_class:  None,
            pause_reason:   None,
            resume_trigger: None,
        }),
    )]).await.unwrap();

    let events_before = store.head_position().await.unwrap();

    let gather  = StandardGatherPhase::builder(store.clone()).build();
    let decide  = StubDecidePhase { response_text: "complete".into() };
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
            &self, _ctx: &OrchestrationContext, _g: &GatherOutput,
        ) -> Result<DecideOutput, OrchestratorError> {
            Ok(DecideOutput {
                raw_response:          "{}".into(),
                proposals:             vec![ActionProposal {
                    action_type:       ActionType::CreateMemory,
                    description:       "store a fact".into(),
                    confidence:        0.5,
                    tool_name:         None,
                    tool_args:         Some(serde_json::json!({ "content": "fact" })),
                    requires_approval: false,
                }],
                calibrated_confidence: 0.5,
                requires_approval:     false,
                model_id:              "stub".into(),
                latency_ms:            0,
            })
        }
    }

    let svc   = setup_run().await;
    let store = svc.store.clone();
    let gather  = StandardGatherPhase::builder(store.clone()).build();
    let decide  = ContinueForeverPhase;
    let execute = build_execute_phase(&svc);
    let config  = LoopConfig { max_iterations: 3, ..LoopConfig::default() };

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
            &self, _ctx: &OrchestrationContext, _g: &GatherOutput,
        ) -> Result<DecideOutput, OrchestratorError> {
            Ok(DecideOutput {
                raw_response:          "{}".into(),
                proposals:             vec![ActionProposal::escalate("need approval", 0.5)],
                calibrated_confidence: 0.5,
                requires_approval:     true,
                model_id:              "stub".into(),
                latency_ms:            0,
            })
        }
    }

    let svc   = setup_run().await;
    let store = svc.store.clone();

    // Transition to Running so approval request succeeds.
    use cairn_domain::{EventEnvelope, EventId, EventSource, RunStateChanged, StateTransition};
    use cairn_domain::lifecycle::RunState;
    store.append(&[EventEnvelope::for_runtime_event(
        EventId::new("evt_appr_e2e"),
        EventSource::Runtime,
        cairn_domain::RuntimeEvent::RunStateChanged(RunStateChanged {
            project:        project(),
            run_id:         run_id(),
            transition:     StateTransition { from: Some(RunState::Pending), to: RunState::Running },
            failure_class:  None,
            pause_reason:   None,
            resume_trigger: None,
        }),
    )]).await.unwrap();

    let gather  = StandardGatherPhase::builder(store.clone()).build();
    let decide  = ApprovalRequiredPhase;
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
/// `qwen/qwen3-coder:free` via OpenRouter.
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
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .expect("set OPENROUTER_API_KEY to run live tests");

    use cairn_runtime::OpenAiCompatProvider;

    let brain_provider = Arc::new(OpenAiCompatProvider::new(
        "https://openrouter.ai/api/v1",
        api_key,
    ));

    // Model: qwen/qwen3-coder:free — 262K context, zero cost.
    let model_id = "qwen/qwen3-coder:free".to_owned();

    let svc   = setup_run().await;
    let store = svc.store.clone();

    // Transition to Running.
    use cairn_domain::{EventEnvelope, EventId, EventSource, RunStateChanged, StateTransition};
    use cairn_domain::lifecycle::RunState;
    store.append(&[EventEnvelope::for_runtime_event(
        EventId::new("evt_live_or_running"),
        EventSource::Runtime,
        cairn_domain::RuntimeEvent::RunStateChanged(RunStateChanged {
            project:        project(),
            run_id:         run_id(),
            transition:     StateTransition { from: Some(RunState::Pending), to: RunState::Running },
            failure_class:  None,
            pause_reason:   None,
            resume_trigger: None,
        }),
    )]).await.unwrap();

    let events_before = store.head_position().await.unwrap();

    let gather  = StandardGatherPhase::builder(store.clone()).build();
    let decide  = cairn_orchestrator::LlmDecidePhase::new(brain_provider, model_id.clone());
    let execute = build_execute_phase(&svc);

    let config = LoopConfig {
        max_iterations: 3,
        timeout_ms:     60_000, // 60-second wall-clock timeout
        ..LoopConfig::default()
    };

    let termination = OrchestratorLoop::new(gather, decide, execute, config)
        .run(base_ctx())
        .await
        .unwrap();

    println!("termination: {termination:?}");

    // Both Completed and MaxIterationsReached are acceptable —
    // the model may finish in one shot or may use all its iterations.
    assert!(
        matches!(
            termination,
            LoopTermination::Completed { .. } | LoopTermination::MaxIterationsReached
        ),
        "expected Completed or MaxIterationsReached, got {termination:?}"
    );

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
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .expect("set OPENROUTER_API_KEY to run live tests");

    let client = reqwest::Client::new();
    let resp = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .expect("request to openrouter.ai/api/v1/models failed");

    assert!(resp.status().is_success(), "expected 200, got {}", resp.status());
    let body: serde_json::Value = resp.json().await.unwrap();
    let models = body["data"].as_array().unwrap();
    assert!(!models.is_empty(), "expected at least one model");

    let free_model = models.iter().any(|m| {
        m["id"].as_str().unwrap_or("").contains("free")
    });
    println!("OpenRouter has free models: {free_model}");
    println!("Total models available: {}", models.len());
}
