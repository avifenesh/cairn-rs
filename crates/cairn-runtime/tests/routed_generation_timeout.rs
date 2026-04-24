//! F27 dogfood blocker: RoutedGenerationService per-call timeout.
//!
//! Even if a provider adapter fails to honour its own timeout (reqwest
//! misconfigured, in-process deadlock, whatever), the routing layer MUST
//! enforce a hard ceiling so the fallback chain still advances. This test
//! pins that belt-and-suspenders contract by scripting a provider that
//! deliberately sleeps past the routing-layer deadline.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cairn_domain::providers::{
    GenerationProvider, GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
};
use cairn_runtime::services::model_chain::ModelChain;
use cairn_runtime::services::routed_generation::{
    RoutedBinding, RoutedGenerationError, RoutedGenerationService,
};

/// Provider that blocks for a fixed duration before returning a (never
/// reached) success. Simulates a broken adapter that ignores its own
/// timeout.
struct HangingProvider {
    sleep: Duration,
}

#[async_trait]
impl GenerationProvider for HangingProvider {
    async fn generate(
        &self,
        model_id: &str,
        _messages: Vec<serde_json::Value>,
        _settings: &ProviderBindingSettings,
        _tools: &[serde_json::Value],
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        tokio::time::sleep(self.sleep).await;
        Ok(GenerationResponse {
            text: "never returned".into(),
            input_tokens: None,
            output_tokens: None,
            model_id: model_id.to_owned(),
            tool_calls: vec![],
            finish_reason: None,
        })
    }
}

/// Provider that succeeds immediately.
struct QuickProvider;

#[async_trait]
impl GenerationProvider for QuickProvider {
    async fn generate(
        &self,
        model_id: &str,
        _messages: Vec<serde_json::Value>,
        _settings: &ProviderBindingSettings,
        _tools: &[serde_json::Value],
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        Ok(GenerationResponse {
            text: "ok".into(),
            input_tokens: Some(1),
            output_tokens: Some(1),
            model_id: model_id.to_owned(),
            tool_calls: vec![],
            finish_reason: Some("stop".into()),
        })
    }
}

/// The routing layer's per-call timeout fires on a hung adapter and the
/// next binding's quick provider takes over. Demonstrates: (a) a hang is
/// classified as `TimedOut`, (b) `TimedOut` is fallback-eligible so the
/// chain advances, (c) the total elapsed time is bounded by the per-call
/// ceiling, not the adapter's sleep.
#[tokio::test]
async fn routed_generation_per_call_timeout_falls_back() {
    let hung = Arc::new(HangingProvider {
        sleep: Duration::from_secs(30),
    });
    let quick = Arc::new(QuickProvider);

    let svc = RoutedGenerationService::new(vec![
        RoutedBinding {
            binding_id: "slow".into(),
            provider: hung,
            chain: ModelChain::single("hung-model"),
        },
        RoutedBinding {
            binding_id: "fast".into(),
            provider: quick,
            chain: ModelChain::single("ok-model"),
        },
    ])
    // 500ms ceiling — strictly less than the 30s sleep so the routing
    // layer must time out, not the adapter.
    .with_per_call_timeout(Duration::from_millis(500));

    let start = std::time::Instant::now();
    let ok = svc
        .generate(vec![], &ProviderBindingSettings::default(), &[])
        .await
        .expect("second binding should succeed");
    let elapsed = start.elapsed();

    assert_eq!(ok.binding_id, "fast");
    assert_eq!(ok.model_id, "ok-model");
    // Elapsed should be ~500ms (the timeout) + epsilon. If it ran the
    // adapter's 30s sleep the test would hang well past this assertion.
    assert!(
        elapsed < Duration::from_secs(5),
        "per-call timeout did not fire: {elapsed:?}"
    );
    // The failed attempt must be recorded as `timed_out`, not some
    // generic transport error — operators triage from `reason_code`.
    assert_eq!(ok.attempts_before_success.len(), 1);
    assert_eq!(ok.attempts_before_success[0].reason_code, "timed_out");
}

/// Every binding hangs → routing layer gives up with
/// `AllProvidersExhausted` in bounded time (N × timeout). This pins the
/// guarantee that a hung provider can never block the orchestrator past
/// the product of (bindings × per-call timeout).
#[tokio::test]
async fn routed_generation_all_hang_exhausts_in_bounded_time() {
    let hung = Arc::new(HangingProvider {
        sleep: Duration::from_secs(30),
    });
    let svc = RoutedGenerationService::new(vec![
        RoutedBinding {
            binding_id: "b1".into(),
            provider: hung.clone(),
            chain: ModelChain::single("m1"),
        },
        RoutedBinding {
            binding_id: "b2".into(),
            provider: hung,
            chain: ModelChain::single("m2"),
        },
    ])
    .with_per_call_timeout(Duration::from_millis(300));

    let start = std::time::Instant::now();
    let err = svc
        .generate(vec![], &ProviderBindingSettings::default(), &[])
        .await
        .unwrap_err();
    let elapsed = start.elapsed();

    // Two bindings × 300ms ceiling = ~600ms. Guard with a 5s envelope
    // (allows CI variance) — ordering-of-magnitude is what matters.
    assert!(
        elapsed < Duration::from_secs(5),
        "exhaustion took too long: {elapsed:?}"
    );
    match err {
        RoutedGenerationError::AllProvidersExhausted { attempts } => {
            assert_eq!(attempts.len(), 2);
            assert!(
                attempts.iter().all(|a| a.reason_code == "timed_out"),
                "attempts: {attempts:?}"
            );
        }
        other => panic!("expected AllProvidersExhausted, got {other:?}"),
    }
}
