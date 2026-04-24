//! Per-provider-binding model fallback chain.
//!
//! # Problem
//!
//! Dogfood run 2 (2026-04-23) revealed that the orchestrator gave up after a
//! single provider error, even when several other free models were available
//! on the same OpenRouter connection. Three attempts in a row died:
//!
//!   1. `minimax/minimax-m2.5:free`  → empty completion (17 whitespace tokens)
//!   2. `qwen/qwen3-coder:free`      → HTTP 503 via OpenRouter → "OpenInference"
//!   3. `meta-llama/llama-3.3-70b-instruct:free` → 429 rate-limit (daily cap)
//!
//! A production control plane must route around flaky upstreams instead of
//! returning `502 decide_error` on the first hiccup.
//!
//! # Design
//!
//! [`ModelChain`] is the **per-binding** axis of fallback: an ordered list of
//! model IDs served by a single provider binding, walked in order on
//! fallback-eligible errors. It composes with the **cross-binding** axis
//! handled by
//! [`crate::services::routed_generation::RoutedGenerationService`]:
//! that service iterates bindings and invokes each binding's `ModelChain`;
//! if the chain exhausts, it advances to the next binding. The two axes
//! remain orthogonal even though they are composed at the
//! `RoutedGenerationService` layer.
//!
//! | Error class                               | Action                                 |
//! |-------------------------------------------|----------------------------------------|
//! | Provider-layer (A): `RateLimited`,        | mark cooldown if RL, advance model     |
//! | 5xx, network, `EmptyResponse`,            |                                        |
//! | `StructuredOutputInvalid`, timeouts       |                                        |
//! | Non-retryable (C): `Auth`, `InvalidRequest`| escalate immediately, no fallback     |
//!
//! Model-action errors (B) — the model emitted a bad tool_call / hallucinated
//! tool name — are **handled by the orchestrator**, not here. Those get
//! looped back into the conversation as tool_result corrections; they do
//! not trigger a model switch on the first offence.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cairn_domain::providers::ProviderAdapterError;

/// Default cooldown applied after a model returns `RateLimited`.
///
/// Kept short (5 minutes) because free-tier daily caps reset at midnight UTC
/// and because the cooldown map lives in-process — a restart clears it. The
/// operator can adjust via [`ModelChain::with_rate_limit_cooldown`].
pub const DEFAULT_RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// A single failed attempt in a model chain.
#[derive(Debug, Clone)]
pub struct FallbackAttempt {
    pub model_id: String,
    pub reason_code: &'static str,
    pub error_message: String,
}

impl FallbackAttempt {
    pub fn new(model_id: &str, err: &ProviderAdapterError) -> Self {
        Self {
            model_id: model_id.to_owned(),
            reason_code: err.reason_code(),
            error_message: err.to_string(),
        }
    }
}

/// Result of walking the fallback chain.
#[derive(Debug)]
pub enum FallbackOutcome<T> {
    /// One of the models succeeded.
    Success {
        value: T,
        model_id: String,
        /// Position in the chain (0 = preferred model).
        fallback_position: usize,
        attempts: Vec<FallbackAttempt>,
    },
    /// Every model in the chain failed with a fallback-eligible error.
    Exhausted { attempts: Vec<FallbackAttempt> },
    /// A model returned an error that MUST NOT be retried on another model
    /// (bad credentials, malformed request).
    NonRetryable {
        model_id: String,
        err: ProviderAdapterError,
        attempts: Vec<FallbackAttempt>,
    },
}

impl<T> FallbackOutcome<T> {
    pub fn attempt_count(&self) -> usize {
        match self {
            FallbackOutcome::Success { attempts, .. }
            | FallbackOutcome::Exhausted { attempts }
            | FallbackOutcome::NonRetryable { attempts, .. } => attempts.len(),
        }
    }
}

/// In-memory cooldown map: model_id → instant until which it should be
/// skipped. Shared across chains so consecutive runs honour the same
/// rate-limit window.
#[derive(Debug, Default, Clone)]
pub struct CooldownMap {
    inner: Arc<Mutex<HashMap<String, Instant>>>,
}

impl CooldownMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, model_id: &str, duration: Duration) {
        let until = Instant::now() + duration;
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(model_id.to_owned(), until);
    }

    pub fn is_cooling_down(&self, model_id: &str) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        match guard.get(model_id).copied() {
            Some(until) if until > now => true,
            Some(_) => {
                guard.remove(model_id);
                false
            }
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        guard.retain(|_, until| *until > now);
        guard.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Per-binding model fallback chain.
///
/// Walks the ordered `models` list on provider-layer errors, skipping
/// cooled-down models. Non-retryable errors (Auth / InvalidRequest)
/// short-circuit with `NonRetryable`.
#[derive(Debug, Clone)]
pub struct ModelChain {
    models: Vec<String>,
    cooldown: CooldownMap,
    rate_limit_cooldown: Duration,
}

impl ModelChain {
    /// Build a chain from an ordered list of model IDs. Empties and
    /// duplicates are dropped. Preserves first-occurrence order.
    pub fn new(models: impl IntoIterator<Item = String>) -> Self {
        let mut deduped: Vec<String> = Vec::new();
        for candidate in models {
            let candidate = candidate.trim();
            if candidate.is_empty() {
                continue;
            }
            if deduped.iter().any(|m| m == candidate) {
                continue;
            }
            deduped.push(candidate.to_owned());
        }
        Self {
            models: deduped,
            cooldown: CooldownMap::new(),
            rate_limit_cooldown: DEFAULT_RATE_LIMIT_COOLDOWN,
        }
    }

    /// Convenience: chain with a single model and no fallbacks.
    pub fn single(model_id: impl Into<String>) -> Self {
        Self::new(std::iter::once(model_id.into()))
    }

    pub fn with_rate_limit_cooldown(mut self, duration: Duration) -> Self {
        self.rate_limit_cooldown = duration;
        self
    }

    pub fn with_cooldown(mut self, map: CooldownMap) -> Self {
        self.cooldown = map;
        self
    }

    pub fn models(&self) -> &[String] {
        &self.models
    }

    pub fn preferred(&self) -> Option<&str> {
        self.models.first().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn rate_limit_cooldown(&self) -> Duration {
        self.rate_limit_cooldown
    }

    /// Run `attempt` for each model in the chain in order, skipping models
    /// currently under cooldown. Returns the first success or the full list
    /// of attempts once the chain is exhausted.
    pub async fn run<T, F, Fut>(&self, mut attempt: F) -> FallbackOutcome<T>
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = Result<T, ProviderAdapterError>>,
    {
        let mut attempts: Vec<FallbackAttempt> = Vec::with_capacity(self.models.len());

        for (position, model_id) in self.models.iter().enumerate() {
            if self.cooldown.is_cooling_down(model_id) {
                attempts.push(FallbackAttempt {
                    model_id: model_id.clone(),
                    reason_code: "cooldown_skip",
                    error_message: format!(
                        "skipped — model under local rate-limit cooldown for {:?}",
                        self.rate_limit_cooldown
                    ),
                });
                continue;
            }

            match attempt(model_id.clone()).await {
                Ok(value) => {
                    return FallbackOutcome::Success {
                        value,
                        model_id: model_id.clone(),
                        fallback_position: position,
                        attempts,
                    };
                }
                Err(err) => {
                    let attempt_record = FallbackAttempt::new(model_id, &err);

                    // Non-retryable (Auth / InvalidRequest) → escalate.
                    if !err.is_fallback_eligible() {
                        attempts.push(attempt_record);
                        return FallbackOutcome::NonRetryable {
                            model_id: model_id.clone(),
                            err,
                            attempts,
                        };
                    }

                    // RateLimited → start cooldown before advancing.
                    if matches!(err, ProviderAdapterError::RateLimited) {
                        self.cooldown.record(model_id, self.rate_limit_cooldown);
                    }

                    attempts.push(attempt_record);
                }
            }
        }

        FallbackOutcome::Exhausted { attempts }
    }
}

/// Format an attempt list into a single human-readable paragraph the
/// app layer can surface via `ToolCallApprovalService::submit_proposal`
/// for operator intervention.
pub fn format_attempt_summary(attempts: &[FallbackAttempt]) -> String {
    if attempts.is_empty() {
        return "no models attempted (fallback chain was empty)".to_owned();
    }
    let mut out = String::with_capacity(attempts.len() * 80);
    out.push_str("all providers failed:\n");
    for (i, a) in attempts.iter().enumerate() {
        out.push_str(&format!(
            "  {}. [{}] {} — {}\n",
            i + 1,
            a.reason_code,
            a.model_id,
            a.error_message
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_rate_limited() -> ProviderAdapterError {
        ProviderAdapterError::RateLimited
    }
    fn err_5xx() -> ProviderAdapterError {
        ProviderAdapterError::ServerError {
            status: 503,
            message: "upstream connect error".to_owned(),
        }
    }
    fn err_empty() -> ProviderAdapterError {
        ProviderAdapterError::EmptyResponse {
            model_id: "m".to_owned(),
            prompt_tokens: Some(500),
            completion_tokens: Some(0),
        }
    }
    fn err_auth() -> ProviderAdapterError {
        ProviderAdapterError::Auth("bad api key".to_owned())
    }
    fn err_invalid() -> ProviderAdapterError {
        ProviderAdapterError::InvalidRequest("unknown field".to_owned())
    }

    #[test]
    fn fallback_eligibility_matrix() {
        assert!(err_rate_limited().is_fallback_eligible());
        assert!(err_5xx().is_fallback_eligible());
        assert!(err_empty().is_fallback_eligible());
        assert!(ProviderAdapterError::StructuredOutputInvalid("x".into()).is_fallback_eligible());
        assert!(!err_auth().is_fallback_eligible());
        assert!(!err_invalid().is_fallback_eligible());
    }

    #[test]
    fn chain_dedupes_and_trims() {
        let chain = ModelChain::new(vec![
            "a".to_owned(),
            "b".to_owned(),
            "a".to_owned(),
            "  ".to_owned(),
            "c".to_owned(),
        ]);
        assert_eq!(chain.models(), &["a", "b", "c"]);
    }

    #[test]
    fn single_model_chain_has_len_one() {
        let chain = ModelChain::single("solo");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain.preferred(), Some("solo"));
    }

    #[test]
    fn empty_chain_is_empty() {
        let empty = ModelChain::new(Vec::<String>::new());
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn chain_succeeds_on_first_model() {
        let chain = ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()]);
        let outcome: FallbackOutcome<String> = chain
            .run(|m| async move { Ok::<_, ProviderAdapterError>(format!("ok-{m}")) })
            .await;
        match outcome {
            FallbackOutcome::Success {
                value,
                model_id,
                fallback_position,
                attempts,
            } => {
                assert_eq!(value, "ok-m1");
                assert_eq!(model_id, "m1");
                assert_eq!(fallback_position, 0);
                assert!(attempts.is_empty());
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn chain_advances_on_rate_limit_then_succeeds() {
        let chain = ModelChain::new(vec!["m1".to_owned(), "m2".to_owned(), "m3".to_owned()]);
        let mut first = true;
        let outcome = chain
            .run(|m| {
                let is_first = first;
                first = false;
                async move {
                    if is_first {
                        Err(err_rate_limited())
                    } else {
                        Ok(format!("ok-{m}"))
                    }
                }
            })
            .await;
        match outcome {
            FallbackOutcome::Success {
                value,
                fallback_position,
                attempts,
                ..
            } => {
                assert_eq!(value, "ok-m2");
                assert_eq!(fallback_position, 1);
                assert_eq!(attempts.len(), 1);
                assert_eq!(attempts[0].reason_code, "rate_limited");
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn chain_advances_on_5xx_then_succeeds() {
        let chain = ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()]);
        let mut first = true;
        let outcome = chain
            .run(|m| {
                let is_first = first;
                first = false;
                async move {
                    if is_first {
                        Err(err_5xx())
                    } else {
                        Ok(format!("ok-{m}"))
                    }
                }
            })
            .await;
        assert!(matches!(outcome, FallbackOutcome::Success { .. }));
    }

    #[tokio::test]
    async fn chain_advances_on_empty_response() {
        let chain = ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()]);
        let mut first = true;
        let outcome = chain
            .run(|m| {
                let is_first = first;
                first = false;
                async move {
                    if is_first {
                        Err(err_empty())
                    } else {
                        Ok(m)
                    }
                }
            })
            .await;
        match outcome {
            FallbackOutcome::Success {
                fallback_position,
                attempts,
                ..
            } => {
                assert_eq!(fallback_position, 1);
                assert_eq!(attempts[0].reason_code, "empty_response");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn chain_escalates_on_auth_without_trying_next() {
        let chain = ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()]);
        let mut calls = 0;
        let outcome: FallbackOutcome<()> = chain
            .run(|_m| {
                calls += 1;
                async move { Err(err_auth()) }
            })
            .await;
        assert_eq!(calls, 1, "must not try m2 after auth error");
        match outcome {
            FallbackOutcome::NonRetryable { model_id, .. } => assert_eq!(model_id, "m1"),
            other => panic!("expected NonRetryable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn chain_escalates_on_invalid_request_without_trying_next() {
        let chain = ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()]);
        let mut calls = 0;
        let outcome: FallbackOutcome<()> = chain
            .run(|_m| {
                calls += 1;
                async move { Err(err_invalid()) }
            })
            .await;
        assert_eq!(calls, 1);
        assert!(matches!(outcome, FallbackOutcome::NonRetryable { .. }));
    }

    #[tokio::test]
    async fn chain_exhausts_when_all_fail_retryably() {
        let chain = ModelChain::new(vec!["m1".to_owned(), "m2".to_owned(), "m3".to_owned()]);
        let outcome: FallbackOutcome<()> = chain.run(|_m| async move { Err(err_5xx()) }).await;
        match outcome {
            FallbackOutcome::Exhausted { attempts } => {
                assert_eq!(attempts.len(), 3);
                for a in attempts {
                    assert_eq!(a.reason_code, "upstream_5xx");
                }
            }
            other => panic!("expected Exhausted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rate_limited_model_skipped_on_next_call() {
        let chain = ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()])
            .with_rate_limit_cooldown(Duration::from_secs(60));

        let mut first = true;
        let _ = chain
            .run(|m| {
                let is_first = first;
                first = false;
                async move {
                    if is_first {
                        Err(err_rate_limited())
                    } else {
                        Ok(m)
                    }
                }
            })
            .await;

        let mut calls: Vec<String> = Vec::new();
        let outcome = chain
            .run(|m| {
                calls.push(m.clone());
                async move { Ok::<_, ProviderAdapterError>(m) }
            })
            .await;
        assert_eq!(calls, vec!["m2".to_owned()], "m1 must be skipped");
        match outcome {
            FallbackOutcome::Success {
                model_id,
                fallback_position,
                attempts,
                ..
            } => {
                assert_eq!(model_id, "m2");
                assert_eq!(fallback_position, 1);
                assert_eq!(attempts.len(), 1);
                assert_eq!(attempts[0].reason_code, "cooldown_skip");
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cooldown_entry_expires() {
        let chain = ModelChain::new(vec!["m1".to_owned(), "m2".to_owned()])
            .with_rate_limit_cooldown(Duration::from_millis(20));
        let _: FallbackOutcome<()> = chain.run(|_m| async move { Err(err_rate_limited()) }).await;
        assert!(chain.cooldown.is_cooling_down("m1"));
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(!chain.cooldown.is_cooling_down("m1"));
    }

    #[test]
    fn format_attempt_summary_lists_each_failure() {
        let attempts = vec![
            FallbackAttempt {
                model_id: "a".to_owned(),
                reason_code: "empty_response",
                error_message: "empty".to_owned(),
            },
            FallbackAttempt {
                model_id: "b".to_owned(),
                reason_code: "upstream_5xx",
                error_message: "503".to_owned(),
            },
        ];
        let s = format_attempt_summary(&attempts);
        assert!(s.contains("empty_response"));
        assert!(s.contains("upstream_5xx"));
        assert!(s.contains("a"));
        assert!(s.contains("b"));
    }

    #[test]
    fn format_attempt_summary_empty_is_graceful() {
        let s = format_attempt_summary(&[]);
        assert!(s.contains("no models"));
    }
}
