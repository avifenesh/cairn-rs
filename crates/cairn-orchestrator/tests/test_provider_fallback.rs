//! Integration tests for the DECIDE-phase routed generation (F17).
//!
//! Reproduces the dogfood run 2 failure modes with a controllable
//! `SequencedProvider` that returns different `ProviderAdapterError`
//! variants per attempt, and asserts that `LlmDecidePhase` walks the
//! composed `RoutedGenerationService` correctly:
//!
//! | Scenario                          | Expected outcome                    |
//! |-----------------------------------|-------------------------------------|
//! | First model 429, second ok        | Success via fallback_position=1     |
//! | First model 503, second ok        | Success via fallback                |
//! | First model empty, second ok      | Success; retry path closes F15      |
//! | First model non-JSON, second ok   | Success via StructuredOutputInvalid |
//! | First model 401                   | ProviderAuthFailed                  |
//! | First model 400                   | ProviderInvalidRequest              |
//! | All three retryable failures      | AllProvidersExhausted w/ 3 attempts |
//! | 429 model skipped on next call    | Cooldown honoured                   |

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cairn_domain::providers::{
    GenerationProvider, GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
};
use cairn_orchestrator::context::{GatherOutput, OrchestrationContext};
use cairn_orchestrator::decide::DecidePhase;
use cairn_orchestrator::{
    CooldownMap, LlmDecidePhase, ModelChain, OrchestratorError, RoutedBinding,
    RoutedGenerationService,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ctx() -> OrchestrationContext {
    OrchestrationContext {
        project: cairn_domain::ProjectKey::new("t", "w", "p"),
        session_id: cairn_domain::SessionId::new("sess_1"),
        run_id: cairn_domain::RunId::new("run_1"),
        task_id: None,
        iteration: 0,
        goal: "Dogfood fallback test.".to_owned(),
        agent_type: "orchestrator".to_owned(),
        run_started_at_ms: 0,
        working_dir: PathBuf::from("."),
        run_mode: cairn_domain::decisions::RunMode::Direct,
        discovered_tool_names: vec![],
        step_history: vec![],
        is_recovery: false,
        approval_timeout: None,
    }
}

fn empty_gather() -> GatherOutput {
    GatherOutput::default()
}

#[derive(Clone)]
enum Programmed {
    Ok(String),
    Err(ProviderAdapterError),
}

struct SequencedProvider {
    scripts: Mutex<std::collections::HashMap<String, Vec<Programmed>>>,
    calls: Mutex<Vec<String>>,
}

impl SequencedProvider {
    fn new(scripts: Vec<(&str, Vec<Programmed>)>) -> Arc<Self> {
        let mut map = std::collections::HashMap::new();
        for (model, outcomes) in scripts {
            map.insert(model.to_owned(), outcomes);
        }
        Arc::new(Self {
            scripts: Mutex::new(map),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl GenerationProvider for SequencedProvider {
    async fn generate(
        &self,
        model_id: &str,
        _messages: Vec<serde_json::Value>,
        _settings: &ProviderBindingSettings,
        _tools: &[serde_json::Value],
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        self.calls.lock().unwrap().push(model_id.to_owned());
        let mut scripts = self.scripts.lock().unwrap();
        let queue = scripts
            .get_mut(model_id)
            .unwrap_or_else(|| panic!("no script programmed for model {model_id}"));
        if queue.is_empty() {
            panic!("no outcomes left for model {model_id}");
        }
        match queue.remove(0) {
            Programmed::Ok(text) => Ok(GenerationResponse {
                text,
                input_tokens: Some(100),
                output_tokens: Some(20),
                model_id: model_id.to_owned(),
                tool_calls: vec![],
                finish_reason: Some("stop".to_owned()),
            }),
            Programmed::Err(e) => Err(e),
        }
    }
}

fn ok_response() -> Programmed {
    Programmed::Ok(
        r#"[{"action_type":"complete_run","description":"done","confidence":0.9,"requires_approval":false}]"#
            .to_owned(),
    )
}

fn err_rate_limited() -> Programmed {
    Programmed::Err(ProviderAdapterError::RateLimited)
}

fn err_5xx() -> Programmed {
    Programmed::Err(ProviderAdapterError::ServerError {
        status: 503,
        message: "upstream connect error".to_owned(),
    })
}

fn err_empty() -> Programmed {
    Programmed::Err(ProviderAdapterError::EmptyResponse {
        model_id: "anything".to_owned(),
        prompt_tokens: Some(500),
        completion_tokens: Some(0),
    })
}

fn err_structured_invalid() -> Programmed {
    Programmed::Err(ProviderAdapterError::StructuredOutputInvalid(
        "not JSON".to_owned(),
    ))
}

fn err_auth() -> Programmed {
    Programmed::Err(ProviderAdapterError::Auth("bad key".to_owned()))
}

fn err_invalid() -> Programmed {
    Programmed::Err(ProviderAdapterError::InvalidRequest(
        "unknown field".to_owned(),
    ))
}

/// Build a single-binding routed service with the given model chain.
fn routed_single_binding(
    provider: Arc<SequencedProvider>,
    models: Vec<&str>,
) -> RoutedGenerationService {
    RoutedGenerationService::new(vec![RoutedBinding {
        binding_id: "b1".to_owned(),
        provider,
        chain: ModelChain::new(models.iter().map(|s| (*s).to_owned())),
    }])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_fallback_on_rate_limited() {
    let provider = SequencedProvider::new(vec![
        ("m1", vec![err_rate_limited()]),
        ("m2", vec![ok_response()]),
    ]);
    let phase = LlmDecidePhase::from_routed(routed_single_binding(
        provider.clone(),
        vec!["m1", "m2"],
    ));
    let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();
    assert_eq!(out.model_id, "m2", "must report the successful model");
    assert_eq!(provider.calls(), vec!["m1", "m2"]);
}

#[tokio::test]
async fn test_fallback_on_5xx() {
    let provider = SequencedProvider::new(vec![
        ("m1", vec![err_5xx()]),
        ("m2", vec![ok_response()]),
    ]);
    let phase = LlmDecidePhase::from_routed(routed_single_binding(
        provider.clone(),
        vec!["m1", "m2"],
    ));
    let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();
    assert_eq!(out.model_id, "m2");
    assert_eq!(provider.calls(), vec!["m1", "m2"]);
}

#[tokio::test]
async fn test_fallback_on_empty_response() {
    let provider = SequencedProvider::new(vec![
        ("m1", vec![err_empty()]),
        ("m2", vec![ok_response()]),
    ]);
    let phase = LlmDecidePhase::from_routed(routed_single_binding(
        provider.clone(),
        vec!["m1", "m2"],
    ));
    let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();
    assert_eq!(out.model_id, "m2");
}

#[tokio::test]
async fn test_fallback_on_response_format() {
    let provider = SequencedProvider::new(vec![
        ("m1", vec![err_structured_invalid()]),
        ("m2", vec![ok_response()]),
    ]);
    let phase = LlmDecidePhase::from_routed(routed_single_binding(
        provider.clone(),
        vec!["m1", "m2"],
    ));
    let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();
    assert_eq!(out.model_id, "m2");
}

#[tokio::test]
async fn test_no_fallback_on_auth_error() {
    let provider = SequencedProvider::new(vec![
        ("m1", vec![err_auth()]),
        ("m2", vec![]), // programmed empty → would panic if reached
    ]);
    let phase = LlmDecidePhase::from_routed(routed_single_binding(
        provider.clone(),
        vec!["m1", "m2"],
    ));
    let err = phase.decide(&ctx(), &empty_gather()).await.unwrap_err();
    match err {
        OrchestratorError::ProviderAuthFailed { model_id, .. } => {
            assert_eq!(model_id, "m1");
        }
        other => panic!("expected ProviderAuthFailed, got {other:?}"),
    }
    assert_eq!(provider.calls(), vec!["m1"], "m2 must not be attempted");
}

#[tokio::test]
async fn test_no_fallback_on_invalid_request() {
    let provider = SequencedProvider::new(vec![
        ("m1", vec![err_invalid()]),
        ("m2", vec![]),
    ]);
    let phase = LlmDecidePhase::from_routed(routed_single_binding(
        provider.clone(),
        vec!["m1", "m2"],
    ));
    let err = phase.decide(&ctx(), &empty_gather()).await.unwrap_err();
    match err {
        OrchestratorError::ProviderInvalidRequest { model_id, .. } => {
            assert_eq!(model_id, "m1");
        }
        other => panic!("expected ProviderInvalidRequest, got {other:?}"),
    }
    assert_eq!(provider.calls(), vec!["m1"]);
}

#[tokio::test]
async fn test_all_providers_exhausted_surfaces_attempt_list() {
    // Reproduces dogfood run 2: MiniMax empty → Qwen 503 → Llama rate-limited.
    let provider = SequencedProvider::new(vec![
        ("minimax", vec![err_empty()]),
        ("qwen", vec![err_5xx()]),
        ("llama", vec![err_rate_limited()]),
    ]);
    let phase = LlmDecidePhase::from_routed(routed_single_binding(
        provider.clone(),
        vec!["minimax", "qwen", "llama"],
    ));

    let err = phase.decide(&ctx(), &empty_gather()).await.unwrap_err();
    match err {
        OrchestratorError::AllProvidersExhausted { attempts } => {
            assert_eq!(attempts.len(), 3);
            assert_eq!(attempts[0].model_id, "minimax");
            assert_eq!(attempts[0].reason_code, "empty_response");
            assert_eq!(attempts[1].model_id, "qwen");
            assert_eq!(attempts[1].reason_code, "upstream_5xx");
            assert_eq!(attempts[2].model_id, "llama");
            assert_eq!(attempts[2].reason_code, "rate_limited");
        }
        other => panic!("expected AllProvidersExhausted, got {other:?}"),
    }
    assert_eq!(provider.calls(), vec!["minimax", "qwen", "llama"]);
}

#[tokio::test]
async fn test_cross_binding_fallback() {
    // First binding's single model fails with 5xx; second binding's model
    // succeeds. This is the cross-binding axis — ProviderRouter territory.
    let p1 = SequencedProvider::new(vec![("a1", vec![err_5xx()])]);
    let p2 = SequencedProvider::new(vec![("b1", vec![ok_response()])]);
    let service = RoutedGenerationService::new(vec![
        RoutedBinding {
            binding_id: "binding-a".into(),
            provider: p1.clone(),
            chain: ModelChain::single("a1"),
        },
        RoutedBinding {
            binding_id: "binding-b".into(),
            provider: p2.clone(),
            chain: ModelChain::single("b1"),
        },
    ]);
    let phase = LlmDecidePhase::from_routed(service);
    let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();
    assert_eq!(out.model_id, "b1");
    assert_eq!(p1.calls(), vec!["a1"]);
    assert_eq!(p2.calls(), vec!["b1"]);
}

#[tokio::test]
async fn test_rate_limited_model_cooldown() {
    let provider = SequencedProvider::new(vec![
        ("m1", vec![err_rate_limited(), ok_response()]),
        ("m2", vec![ok_response(), ok_response()]),
    ]);
    // Share cooldown map between two consecutive phase instantiations,
    // mirroring AppState::provider_fallback_cooldown in production.
    let cooldown = CooldownMap::new();
    let build_phase = || {
        let chain = ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()])
            .with_rate_limit_cooldown(Duration::from_secs(60))
            .with_cooldown(cooldown.clone());
        let service = RoutedGenerationService::new(vec![RoutedBinding {
            binding_id: "b1".into(),
            provider: provider.clone(),
            chain,
        }]);
        LlmDecidePhase::from_routed(service)
    };

    let out1 = build_phase().decide(&ctx(), &empty_gather()).await.unwrap();
    assert_eq!(out1.model_id, "m2");
    assert_eq!(provider.calls(), vec!["m1", "m2"]);

    let out2 = build_phase().decide(&ctx(), &empty_gather()).await.unwrap();
    assert_eq!(out2.model_id, "m2");
    assert_eq!(
        provider.calls(),
        vec!["m1", "m2", "m2"],
        "second call must skip cooled-down m1"
    );
}

#[tokio::test]
async fn test_single_model_chain_preserves_legacy_behaviour() {
    let provider = SequencedProvider::new(vec![("solo", vec![ok_response()])]);
    // `LlmDecidePhase::new` wraps a single provider in a one-binding chain.
    let phase = LlmDecidePhase::new(provider.clone(), "solo");
    let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();
    assert_eq!(out.model_id, "solo");
}
