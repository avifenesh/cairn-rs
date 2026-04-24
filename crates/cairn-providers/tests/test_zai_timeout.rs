//! F27 dogfood blocker: Z.ai timeout regression tests.
//!
//! Motivation: the live dogfood run hung for 5+ minutes when Z.ai stopped
//! responding. Root cause was `timeout_secs: None` flowing through
//! `ZaiProvider::new` unchanged and reqwest's default client timeout being
//! effectively infinite. These tests pin the two contracts that fix the hang:
//!
//! 1. With no explicit timeout, the client MUST still enforce a bounded
//!    default — a slow upstream surfaces as `ProviderError::TimedOut`, not
//!    an unbounded await.
//! 2. Explicit timeouts still win (operator override).
//!
//! We use `httpmock`'s built-in delay to simulate a hung upstream: setting
//! the provider timeout to 1s and the upstream delay to 10s makes the
//! timeout deterministic without hanging CI.

use std::time::Duration;

use cairn_providers::{
    ChatMessage, ChatProvider, ChatRole, MessageContent,
    error::ProviderError,
    wire::zai::{ZaiConfig, ZaiProvider},
};
use httpmock::prelude::*;

fn user_msg() -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: ChatRole::User,
        content: "hi".to_owned(),
        content_type: MessageContent::Text,
    }]
}

/// A provider-supplied `timeout_secs` fires and surfaces as
/// `ProviderError::TimedOut`. Baseline contract — if this breaks, the
/// F27 fix is gone.
#[tokio::test]
async fn zai_provider_timeout_fires_and_surfaces_as_timedout() {
    let server = MockServer::start();
    // httpmock holds the response for 10s — far longer than the 1s cap.
    let _mock = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .delay(Duration::from_secs(10))
            .header("content-type", "application/json")
            .body(r#"{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#);
    });

    let provider = ZaiProvider::new(
        ZaiConfig::CODING,
        "test-key",
        Some(server.base_url()),
        Some("glm-4.7".to_owned()),
        None,
        None,
        Some(1), // 1-second cap
    )
    .expect("provider build");

    let start = std::time::Instant::now();
    let result = provider.chat_with_tools(&user_msg(), None, None).await;
    let elapsed = start.elapsed();

    assert!(
        matches!(result, Err(ProviderError::TimedOut)),
        "expected ProviderError::TimedOut, got {result:?}"
    );
    // Sanity: completed well under the mock's 10s delay.
    assert!(
        elapsed < Duration::from_secs(5),
        "timeout did not short-circuit: {elapsed:?}"
    );
}

/// When the caller passes `None` for `timeout_secs`, the provider must
/// still install a bounded default from `ZaiConfig::default_timeout_secs`.
/// We can't feasibly wait 120s in a test, so we verify the contract by
/// constructing a config with a tiny default and confirming it fires.
#[tokio::test]
async fn zai_provider_uses_config_default_when_no_explicit_timeout() {
    let server = MockServer::start();
    let _mock = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .delay(Duration::from_secs(10))
            .header("content-type", "application/json")
            .body(r#"{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#);
    });

    // Custom config with a 1s default timeout — same shape as CODING but
    // short enough for CI. Confirms the `None` → default path fires.
    let config = ZaiConfig {
        default_timeout_secs: 1,
        ..ZaiConfig::CODING
    };
    let provider = ZaiProvider::new(
        config,
        "test-key",
        Some(server.base_url()),
        Some("glm-4.7".to_owned()),
        None,
        None,
        None, // no explicit override — must fall back to config default
    )
    .expect("provider build");

    let start = std::time::Instant::now();
    let result = provider.chat_with_tools(&user_msg(), None, None).await;
    let elapsed = start.elapsed();

    assert!(
        matches!(result, Err(ProviderError::TimedOut)),
        "expected ProviderError::TimedOut from config default, got {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "config-default timeout did not short-circuit: {elapsed:?}"
    );
}

/// Regression: the shipped `ZaiConfig::CODING` MUST carry a bounded default.
/// Zero / missing would re-introduce the F27 hang.
#[test]
fn zai_coding_config_has_bounded_default_timeout() {
    // Both presets match the module constant (no accidental divergence
    // between tiers; a zero-value default would reintroduce F27).
    assert_eq!(
        ZaiConfig::CODING.default_timeout_secs,
        cairn_providers::wire::zai::DEFAULT_TIMEOUT_SECS,
    );
    assert_eq!(
        ZaiConfig::GENERAL.default_timeout_secs,
        cairn_providers::wire::zai::DEFAULT_TIMEOUT_SECS,
    );
    // The module-level constant itself must be bounded-and-positive
    // (zero would re-introduce F27). `const` block so clippy is happy
    // about asserting on a constant value.
    const { assert!(cairn_providers::wire::zai::DEFAULT_TIMEOUT_SECS != 0) };
}
