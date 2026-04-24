//! Composed routing service for LLM generation.
//!
//! Combines the two fallback axes — cross-binding and per-binding — into
//! a single entry point the orchestrator can call.
//!
//! ```text
//!  RoutedGenerationService::generate
//!      │
//!      ├── for each binding in `bindings` (cross-binding order):
//!      │       │
//!      │       └── for each model in binding's ModelChain:
//!      │               │
//!      │               ├── provider.generate(model, ..., tools)
//!      │               │      ├── Ok              → return Success
//!      │               │      ├── Auth / Invalid  → short-circuit (C)
//!      │               │      └── retryable (A)   → advance to next model
//!      │
//!      └── all bindings × models exhausted → AllProvidersExhausted(attempts)
//! ```
//!
//! # Error taxonomy (matches dogfood run 2 user direction)
//!
//! - **Provider-layer errors (A)**: `RateLimited`, 5xx, network, `EmptyResponse`,
//!   `StructuredOutputInvalid`, transport timeouts. Logged at WARN. Advance
//!   to the next model in the current binding's chain, then to the next
//!   binding. Returns [`RoutedGenerationError::AllProvidersExhausted`] when
//!   the full product of bindings × models fails.
//! - **Non-retryable (C)**: `Auth`, `InvalidRequest`. Logged at ERROR. Short
//!   circuits with [`RoutedGenerationError::Auth`] or
//!   [`RoutedGenerationError::InvalidRequest`].
//! - **Model-action errors (B)**: not this layer's concern. Lives in the
//!   orchestrator decide phase (bad tool_call names / args get replayed into
//!   the conversation as `tool_result` corrections).
//!
//! # Logging
//!
//! Every dispatch attempt emits a `tracing` span with structured fields
//! (`binding_id`, `model_id`, `attempt_index`). Prompt / response bodies
//! are owned by the caller (the decide phase, which already logs
//! `LlmCallTrace` records via `LlmObservabilityServiceImpl`).

use std::sync::Arc;

use cairn_domain::providers::{
    GenerationProvider, GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
};

use super::model_chain::{
    CooldownMap, FallbackAttempt, FallbackOutcome, ModelChain, DEFAULT_RATE_LIMIT_COOLDOWN,
};

/// One provider binding that can be tried as part of the cross-binding
/// fallback chain.
///
/// `chain` lists the models served by the underlying provider connection
/// in the order the operator wants them attempted. `provider` is the
/// resolved `GenerationProvider` for the binding (typically built by
/// `ProviderRegistry::resolve_generation_for_model`).
#[derive(Clone)]
pub struct RoutedBinding {
    /// Human-readable binding/connection identifier for logs and attempt
    /// records. Typically the provider-connection ID.
    pub binding_id: String,
    /// Generation provider adapter. The service calls
    /// `provider.generate(model, ...)` for each model in `chain`.
    pub provider: Arc<dyn GenerationProvider>,
    /// Models to try on this binding, in order.
    pub chain: ModelChain,
}

/// Composed routing service. Walks cross-binding chain, then per-binding
/// model chain, forwarding tool definitions to every call.
#[derive(Clone)]
pub struct RoutedGenerationService {
    bindings: Vec<RoutedBinding>,
}

impl RoutedGenerationService {
    pub fn new(bindings: Vec<RoutedBinding>) -> Self {
        Self { bindings }
    }

    /// True when the service has no bindings at all — callers should surface
    /// a 503 "no provider configured" instead of attempting routing.
    pub fn is_empty(&self) -> bool {
        self.bindings.iter().all(|b| b.chain.is_empty())
    }

    /// Total number of model attempts the full cross-binding × per-binding
    /// walk could perform (ignoring cooldowns). Useful for metrics.
    pub fn attempt_budget(&self) -> usize {
        self.bindings.iter().map(|b| b.chain.len()).sum()
    }

    /// Walk the composed chain until a model succeeds or the chain is
    /// exhausted.
    ///
    /// `tools` is forwarded to every provider call — fixes the prior bug
    /// where `ProviderRouter::route` passed `&[]` and silently dropped the
    /// orchestrator's tool catalog.
    pub async fn generate(
        &self,
        messages: Vec<serde_json::Value>,
        settings: &ProviderBindingSettings,
        tools: &[serde_json::Value],
    ) -> Result<RoutedGenerationSuccess, RoutedGenerationError> {
        let mut all_attempts: Vec<FallbackAttempt> = Vec::with_capacity(self.attempt_budget());

        if self.bindings.is_empty() {
            return Err(RoutedGenerationError::AllProvidersExhausted {
                attempts: all_attempts,
            });
        }

        for (binding_idx, binding) in self.bindings.iter().enumerate() {
            if binding.chain.is_empty() {
                continue;
            }

            let provider = binding.provider.clone();
            let binding_id = binding.binding_id.clone();
            let settings_owned = settings.clone();
            let messages_owned = messages.clone();
            let tools_owned = tools.to_vec();

            let outcome: FallbackOutcome<GenerationResponse> = binding
                .chain
                .run(|model_id| {
                    let provider = provider.clone();
                    let settings = settings_owned.clone();
                    let msgs = messages_owned.clone();
                    let tools = tools_owned.clone();
                    let binding_id = binding_id.clone();
                    let attempt_idx = all_attempts.len();
                    async move {
                        tracing::info!(
                            binding_id = %binding_id,
                            model_id = %model_id,
                            attempt_index = attempt_idx,
                            tool_count = tools.len(),
                            "routed_generation: dispatch attempt"
                        );
                        let result = provider.generate(&model_id, msgs, &settings, &tools).await;
                        match &result {
                            Ok(resp) => {
                                tracing::info!(
                                    binding_id = %binding_id,
                                    model_id = %model_id,
                                    input_tokens = ?resp.input_tokens,
                                    output_tokens = ?resp.output_tokens,
                                    tool_calls = resp.tool_calls.len(),
                                    "routed_generation: success"
                                );
                            }
                            Err(err) => {
                                tracing::warn!(
                                    binding_id = %binding_id,
                                    model_id = %model_id,
                                    reason = err.reason_code(),
                                    error = %err,
                                    "routed_generation: dispatch failed"
                                );
                            }
                        }
                        result
                    }
                })
                .await;

            match outcome {
                FallbackOutcome::Success {
                    value,
                    model_id,
                    fallback_position,
                    attempts,
                } => {
                    all_attempts.extend(attempts);
                    return Ok(RoutedGenerationSuccess {
                        response: value,
                        binding_id: binding.binding_id.clone(),
                        binding_index: binding_idx,
                        model_id,
                        fallback_position,
                        attempts_before_success: all_attempts,
                    });
                }
                FallbackOutcome::NonRetryable {
                    model_id,
                    err,
                    attempts,
                } => {
                    all_attempts.extend(attempts);
                    let detail = err.to_string();
                    return Err(match err {
                        ProviderAdapterError::Auth(_) => RoutedGenerationError::Auth {
                            binding_id: binding.binding_id.clone(),
                            model_id,
                            detail,
                            attempts: all_attempts,
                        },
                        _ => RoutedGenerationError::InvalidRequest {
                            binding_id: binding.binding_id.clone(),
                            model_id,
                            detail,
                            attempts: all_attempts,
                        },
                    });
                }
                FallbackOutcome::Exhausted { attempts } => {
                    all_attempts.extend(attempts);
                    // Advance to the next binding.
                    continue;
                }
            }
        }

        Err(RoutedGenerationError::AllProvidersExhausted {
            attempts: all_attempts,
        })
    }
}

/// Successful routed generation. Carries provenance so downstream LLM
/// observability records reflect which binding/model actually served the
/// call, not the caller's preferred choice.
#[derive(Debug, Clone)]
pub struct RoutedGenerationSuccess {
    pub response: GenerationResponse,
    pub binding_id: String,
    pub binding_index: usize,
    pub model_id: String,
    /// 0 = first model on first binding ; > 0 = some fallback occurred.
    pub fallback_position: usize,
    /// Every attempt recorded **before** the successful one (skipped
    /// cooldowns + failures). Empty when the first model succeeds.
    pub attempts_before_success: Vec<FallbackAttempt>,
}

/// Error outcomes for [`RoutedGenerationService::generate`].
#[derive(Debug, Clone)]
pub enum RoutedGenerationError {
    /// Every binding × model combination failed with fallback-eligible
    /// errors. The caller should surface the full attempt list so the
    /// operator can see which providers failed and why.
    AllProvidersExhausted { attempts: Vec<FallbackAttempt> },
    /// Non-retryable authentication failure. Subsequent attempts aborted
    /// because retrying with another model on the same binding would hit
    /// the same credential problem.
    Auth {
        binding_id: String,
        model_id: String,
        detail: String,
        attempts: Vec<FallbackAttempt>,
    },
    /// Non-retryable malformed-request failure.
    InvalidRequest {
        binding_id: String,
        model_id: String,
        detail: String,
        attempts: Vec<FallbackAttempt>,
    },
}

impl RoutedGenerationError {
    pub fn attempts(&self) -> &[FallbackAttempt] {
        match self {
            RoutedGenerationError::AllProvidersExhausted { attempts }
            | RoutedGenerationError::Auth { attempts, .. }
            | RoutedGenerationError::InvalidRequest { attempts, .. } => attempts,
        }
    }

    /// Short machine-readable code for error responses.
    pub fn code(&self) -> &'static str {
        match self {
            RoutedGenerationError::AllProvidersExhausted { .. } => "all_providers_exhausted",
            RoutedGenerationError::Auth { .. } => "provider_auth_failed",
            RoutedGenerationError::InvalidRequest { .. } => "provider_invalid_request",
        }
    }
}

impl std::fmt::Display for RoutedGenerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutedGenerationError::AllProvidersExhausted { attempts } => {
                write!(
                    f,
                    "{}",
                    super::model_chain::format_attempt_summary(attempts)
                )
            }
            RoutedGenerationError::Auth {
                binding_id,
                model_id,
                detail,
                ..
            } => write!(
                f,
                "provider auth failed on binding={binding_id} model={model_id}: {detail}"
            ),
            RoutedGenerationError::InvalidRequest {
                binding_id,
                model_id,
                detail,
                ..
            } => write!(
                f,
                "provider rejected request on binding={binding_id} model={model_id}: {detail}"
            ),
        }
    }
}

impl std::error::Error for RoutedGenerationError {}

/// Convenience: a single-binding, single-model routing service. Used by
/// the legacy call sites that haven't migrated to real chains yet.
pub fn single_model_service(
    binding_id: impl Into<String>,
    provider: Arc<dyn GenerationProvider>,
    model_id: impl Into<String>,
) -> RoutedGenerationService {
    RoutedGenerationService::new(vec![RoutedBinding {
        binding_id: binding_id.into(),
        provider,
        chain: ModelChain::single(model_id),
    }])
}

/// Expose the default cooldown constant for callers that want to align
/// with the model-chain defaults.
pub const DEFAULT_COOLDOWN: std::time::Duration = DEFAULT_RATE_LIMIT_COOLDOWN;

// Re-export for callers reaching us via `cairn_runtime::services::*`.
pub use super::model_chain::CooldownMap as RoutedCooldownMap;

// Prevent "unused" warning when CooldownMap import used only via re-export.
#[allow(dead_code)]
fn _touch_cooldown_map(_m: &CooldownMap) {}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Scripted provider that returns a fixed sequence of results per model.
    struct ScriptedProvider {
        script: Mutex<std::collections::HashMap<String, Vec<ScriptStep>>>,
        /// Records every (model_id, tools_len) call — lets us assert tools
        /// are forwarded.
        calls: Mutex<Vec<(String, usize)>>,
    }

    #[derive(Clone)]
    enum ScriptStep {
        Ok(String),
        Err(ProviderAdapterError),
    }

    impl ScriptedProvider {
        fn new(script: Vec<(&'static str, Vec<ScriptStep>)>) -> Arc<Self> {
            let mut map = std::collections::HashMap::new();
            for (k, v) in script {
                map.insert(k.to_owned(), v);
            }
            Arc::new(Self {
                script: Mutex::new(map),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn calls(&self) -> Vec<(String, usize)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl GenerationProvider for ScriptedProvider {
        async fn generate(
            &self,
            model_id: &str,
            _messages: Vec<serde_json::Value>,
            _settings: &ProviderBindingSettings,
            tools: &[serde_json::Value],
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            self.calls
                .lock()
                .unwrap()
                .push((model_id.to_owned(), tools.len()));
            let step = {
                let mut guard = self.script.lock().unwrap();
                let steps = guard.get_mut(model_id).ok_or_else(|| {
                    ProviderAdapterError::InvalidRequest(format!(
                        "scripted: no steps for {model_id}"
                    ))
                })?;
                if steps.is_empty() {
                    return Err(ProviderAdapterError::InvalidRequest(format!(
                        "scripted: exhausted steps for {model_id}"
                    )));
                }
                steps.remove(0)
            };
            match step {
                ScriptStep::Ok(text) => Ok(GenerationResponse {
                    text,
                    input_tokens: Some(10),
                    output_tokens: Some(5),
                    model_id: model_id.to_owned(),
                    tool_calls: vec![],
                    finish_reason: Some("stop".to_owned()),
                }),
                ScriptStep::Err(e) => Err(e),
            }
        }
    }

    fn settings() -> ProviderBindingSettings {
        ProviderBindingSettings::default()
    }

    fn tools() -> Vec<serde_json::Value> {
        vec![serde_json::json!({
            "type": "function",
            "function": { "name": "search", "description": "d", "parameters": {} }
        })]
    }

    #[tokio::test]
    async fn test_primary_binding_first_model_success() {
        let p = ScriptedProvider::new(vec![("m1", vec![ScriptStep::Ok("hello".into())])]);
        let svc = RoutedGenerationService::new(vec![RoutedBinding {
            binding_id: "b1".into(),
            provider: p.clone(),
            chain: ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()]),
        }]);
        let ok = svc.generate(vec![], &settings(), &tools()).await.unwrap();
        assert_eq!(ok.model_id, "m1");
        assert_eq!(ok.binding_id, "b1");
        assert_eq!(ok.fallback_position, 0);
        assert!(ok.attempts_before_success.is_empty());
        // Tool forwarding bug fix: provider saw the tool definitions.
        assert_eq!(p.calls(), vec![("m1".to_owned(), 1)]);
    }

    #[tokio::test]
    async fn test_first_model_rate_limited_second_model_success() {
        let p = ScriptedProvider::new(vec![
            (
                "m1",
                vec![ScriptStep::Err(ProviderAdapterError::RateLimited)],
            ),
            ("m2", vec![ScriptStep::Ok("hi".into())]),
        ]);
        let svc = RoutedGenerationService::new(vec![RoutedBinding {
            binding_id: "b1".into(),
            provider: p.clone(),
            chain: ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()]),
        }]);
        let ok = svc.generate(vec![], &settings(), &tools()).await.unwrap();
        assert_eq!(ok.model_id, "m2");
        assert_eq!(ok.fallback_position, 1);
        assert_eq!(ok.attempts_before_success.len(), 1);
        assert_eq!(ok.attempts_before_success[0].reason_code, "rate_limited");
    }

    #[tokio::test]
    async fn test_first_binding_all_models_exhausted_second_binding_succeeds() {
        let p1 = ScriptedProvider::new(vec![
            (
                "a1",
                vec![ScriptStep::Err(ProviderAdapterError::RateLimited)],
            ),
            (
                "a2",
                vec![ScriptStep::Err(ProviderAdapterError::ServerError {
                    status: 503,
                    message: "down".into(),
                })],
            ),
        ]);
        let p2 = ScriptedProvider::new(vec![("b1", vec![ScriptStep::Ok("ok".into())])]);
        let svc = RoutedGenerationService::new(vec![
            RoutedBinding {
                binding_id: "binding-a".into(),
                provider: p1,
                chain: ModelChain::new(vec!["a1".to_owned(), "a2".to_owned()]),
            },
            RoutedBinding {
                binding_id: "binding-b".into(),
                provider: p2,
                chain: ModelChain::new(vec!["b1".to_owned()]),
            },
        ]);
        let ok = svc.generate(vec![], &settings(), &tools()).await.unwrap();
        assert_eq!(ok.binding_id, "binding-b");
        assert_eq!(ok.binding_index, 1);
        assert_eq!(ok.model_id, "b1");
        assert_eq!(ok.attempts_before_success.len(), 2);
    }

    #[tokio::test]
    async fn test_auth_error_returns_immediately_no_fallback() {
        let p = ScriptedProvider::new(vec![(
            "m1",
            vec![ScriptStep::Err(ProviderAdapterError::Auth(
                "bad key".into(),
            ))],
        )]);
        let svc = RoutedGenerationService::new(vec![
            RoutedBinding {
                binding_id: "b1".into(),
                provider: p.clone(),
                chain: ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()]),
            },
            RoutedBinding {
                binding_id: "b2".into(),
                provider: ScriptedProvider::new(vec![("z", vec![ScriptStep::Ok("no".into())])]),
                chain: ModelChain::single("z"),
            },
        ]);
        let err = svc
            .generate(vec![], &settings(), &tools())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "provider_auth_failed");
        // Only m1 was called; m2 and b2/z never got a request.
        assert_eq!(p.calls().len(), 1);
    }

    #[tokio::test]
    async fn test_invalid_request_returns_immediately_no_fallback() {
        let p = ScriptedProvider::new(vec![(
            "m1",
            vec![ScriptStep::Err(ProviderAdapterError::InvalidRequest(
                "bad param".into(),
            ))],
        )]);
        let svc = RoutedGenerationService::new(vec![RoutedBinding {
            binding_id: "b1".into(),
            provider: p.clone(),
            chain: ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()]),
        }]);
        let err = svc
            .generate(vec![], &settings(), &tools())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "provider_invalid_request");
        assert_eq!(p.calls().len(), 1);
    }

    #[tokio::test]
    async fn test_all_providers_exhausted_returns_attempts_vec() {
        let p = ScriptedProvider::new(vec![
            (
                "m1",
                vec![ScriptStep::Err(ProviderAdapterError::ServerError {
                    status: 502,
                    message: "x".into(),
                })],
            ),
            (
                "m2",
                vec![ScriptStep::Err(ProviderAdapterError::EmptyResponse {
                    model_id: "m2".into(),
                    prompt_tokens: Some(10),
                    completion_tokens: Some(0),
                })],
            ),
        ]);
        let svc = RoutedGenerationService::new(vec![RoutedBinding {
            binding_id: "b1".into(),
            provider: p,
            chain: ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()]),
        }]);
        let err = svc
            .generate(vec![], &settings(), &tools())
            .await
            .unwrap_err();
        match err {
            RoutedGenerationError::AllProvidersExhausted { attempts } => {
                assert_eq!(attempts.len(), 2);
                assert_eq!(attempts[0].reason_code, "upstream_5xx");
                assert_eq!(attempts[1].reason_code, "empty_response");
            }
            other => panic!("expected AllProvidersExhausted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_rate_limited_cooldown_skips_model_for_cooldown_window() {
        use std::time::Duration;
        let cooldown = CooldownMap::new();
        let p = ScriptedProvider::new(vec![
            // First call to m1 returns rate-limited, then m2 succeeds.
            (
                "m1",
                vec![ScriptStep::Err(ProviderAdapterError::RateLimited)],
            ),
            (
                "m2",
                vec![ScriptStep::Ok("a".into()), ScriptStep::Ok("b".into())],
            ),
        ]);
        let svc = RoutedGenerationService::new(vec![RoutedBinding {
            binding_id: "b1".into(),
            provider: p.clone(),
            chain: ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()])
                .with_rate_limit_cooldown(Duration::from_secs(60))
                .with_cooldown(cooldown.clone()),
        }]);

        // First call: m1 rate-limits, m2 succeeds. m1 is now cooled down.
        let _ = svc.generate(vec![], &settings(), &tools()).await.unwrap();

        // Second call on the same service (shared cooldown map): m1 is
        // skipped before dispatch, m2 is called directly.
        let ok2 = svc.generate(vec![], &settings(), &tools()).await.unwrap();
        assert_eq!(ok2.model_id, "m2");
        // p's recorded calls: m1 (failed), m2 (first call), m2 (second call)
        let calls = p.calls();
        assert_eq!(
            calls.iter().filter(|(m, _)| m == "m1").count(),
            1,
            "m1 must not be retried during cooldown"
        );
        assert_eq!(calls.iter().filter(|(m, _)| m == "m2").count(), 2);
    }

    #[tokio::test]
    async fn test_tool_defs_forwarded_to_provider() {
        // Regression for the `&[]` bug in ProviderRouter::route.
        let p = ScriptedProvider::new(vec![("m1", vec![ScriptStep::Ok("ok".into())])]);
        let svc = RoutedGenerationService::new(vec![RoutedBinding {
            binding_id: "b1".into(),
            provider: p.clone(),
            chain: ModelChain::single("m1"),
        }]);
        let tool_defs = vec![
            serde_json::json!({ "type": "function", "function": { "name": "read" } }),
            serde_json::json!({ "type": "function", "function": { "name": "write" } }),
        ];
        let _ = svc.generate(vec![], &settings(), &tool_defs).await.unwrap();
        assert_eq!(
            p.calls(),
            vec![("m1".to_owned(), 2)],
            "provider must receive the tool defs we passed in"
        );
    }

    #[tokio::test]
    async fn test_empty_bindings_returns_exhausted() {
        let svc = RoutedGenerationService::new(vec![]);
        let err = svc.generate(vec![], &settings(), &[]).await.unwrap_err();
        assert_eq!(err.code(), "all_providers_exhausted");
        assert!(err.attempts().is_empty());
    }
}
