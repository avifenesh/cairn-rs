//! Real-provider contract test for OpenRouter free-tier chat completions.
//!
//! **Default mode (CI, no env vars):** loads a checked-in JSON fixture at
//! `tests/fixtures/openrouter_chat_completion.json`, serves it via a local
//! mock HTTP server, and exercises the real `OpenAiCompat` provider path
//! (configured with `ProviderConfig::OPENROUTER`).  This is deliberately
//! the same parser code path the orchestrator uses in production — if the
//! OpenRouter response shape drifts in a way that breaks field extraction,
//! this test fails.
//!
//! **Refresh mode (`CAIRN_TEST_REFRESH_FIXTURES=1` + `OPENROUTER_API_KEY`
//! set):** constructs a canonical chat request, POSTs it to the real
//! OpenRouter API, and overwrites the fixture with the raw response body.
//! The refreshed fixture should then be committed to git.
//!
//! # Scope
//!
//! This is the first of three planned reality-check tests for provider
//! integration: this contract test (provider shape drift), then chaos
//! tests (failure-mode behavior), then soak tests (long-running stability).
//! Other providers (Anthropic-native, Bedrock Converse, Vertex, etc.) will
//! get their own contract tests in follow-up PRs using the same pattern.
//!
//! # Why OpenRouter first
//!
//! - Cheap — free-tier models cost $0 per request.
//! - OpenAI-compatible wire format — exercises the shared parser used by
//!   many backends (OpenAI, Groq, DeepSeek, xAI, MiniMax, etc.).
//! - Broad model selection — operator can pick any `:free` model.
//! - Acceptable to fail noisily when routes change — free models are the
//!   most likely to be deprecated or rename, giving us early warning.
//!
//! # Refreshing the fixture
//!
//! ```bash
//! OPENROUTER_API_KEY=<key> CAIRN_TEST_REFRESH_FIXTURES=1 \
//!     cargo test -p cairn-providers --test contract_openrouter -- --nocapture
//! ```
//!
//! See `tests/README.md` for the full refresh workflow and cadence.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use cairn_providers::{
    ChatMessage, ChatProvider,
    wire::openai_compat::{OpenAiCompat, ProviderConfig},
};
use httpmock::prelude::*;

/// Canonical free model used for refresh requests.  If this is ever
/// deprecated by OpenRouter, pick another `:free` model from
/// <https://openrouter.ai/models?q=free> and update this constant plus
/// the `model` field in the fixture.
const REFRESH_MODEL: &str = "meta-llama/llama-3.2-3b-instruct:free";

/// Canonical user prompt for refresh requests.  Short to minimize
/// response size (and cost, though free-tier is $0).
const REFRESH_PROMPT: &str = "Say hello in one word.";

/// Max tokens for refresh requests.  Small to keep fixtures compact.
const REFRESH_MAX_TOKENS: u32 = 50;

/// Freshness warning threshold.  Fixtures older than this print an
/// informational WARN line but do **not** fail the test — refresh cadence
/// is operator discretion.
const FRESHNESS_WARN_AFTER: Duration = Duration::from_secs(90 * 24 * 60 * 60);

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("openrouter_chat_completion.json")
}

/// Build the provider pointed at the given base URL.  Used both for the
/// real refresh path (OpenRouter URL) and the mock replay path (httpmock
/// URL).  Using the same constructor in both modes guarantees we exercise
/// the identical parser code path production uses.
fn build_provider(base_url: &str, api_key: &str) -> OpenAiCompat {
    OpenAiCompat::new(
        ProviderConfig::OPENROUTER,
        api_key,
        Some(base_url.to_owned()),
        Some(REFRESH_MODEL.to_owned()),
        Some(REFRESH_MAX_TOKENS),
        Some(0.0),
        Some(30),
    )
    .expect("OpenAiCompat should build from valid config")
}

/// Refresh mode — POST a real request to OpenRouter and persist the raw
/// response body to the fixture file.  Uses plain `reqwest` (not the
/// provider) so we capture the body byte-for-byte, not the parsed shape.
async fn refresh_fixture(api_key: &str) -> String {
    let url = format!(
        "{}{}",
        ProviderConfig::OPENROUTER.default_base_url,
        ProviderConfig::OPENROUTER.chat_endpoint
    );
    let body = serde_json::json!({
        "model": REFRESH_MODEL,
        "messages": [{"role": "user", "content": REFRESH_PROMPT}],
        "max_tokens": REFRESH_MAX_TOKENS,
        "temperature": 0.0,
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("reqwest client should build");
    let resp = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .expect("OpenRouter refresh request should succeed");
    let status = resp.status();
    let text = resp
        .text()
        .await
        .expect("OpenRouter response body should decode as UTF-8");
    assert!(
        status.is_success(),
        "OpenRouter returned HTTP {status}: {text}"
    );
    text
}

/// Default replay mode — serve the fixture body through a local mock HTTP
/// server and invoke the real provider.  Asserts every field the
/// orchestrator reads (id is optional in our parser, so we focus on
/// content, finish_reason, usage).
#[tokio::test]
async fn openrouter_chat_completion_response_shape_matches_fixture() {
    let path = fixture_path();

    // Refresh mode — hit OpenRouter for real and overwrite the fixture.
    if env::var("CAIRN_TEST_REFRESH_FIXTURES").is_ok() {
        let api_key = env::var("OPENROUTER_API_KEY").expect(
            "refresh mode requires OPENROUTER_API_KEY — unset CAIRN_TEST_REFRESH_FIXTURES to run replay-only",
        );
        let body = refresh_fixture(&api_key).await;
        // Pretty-print so diffs in git reviews are readable.
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("OpenRouter response should be valid JSON");
        let pretty =
            serde_json::to_string_pretty(&parsed).expect("serde_json re-serialize should succeed");
        fs::write(&path, format!("{pretty}\n")).expect("fixture write should succeed");
        eprintln!("[REFRESHED] {}", path.display());
    }

    // Replay mode — fixture must exist.
    if !path.exists() {
        panic!(
            "fixture missing at {} — run with CAIRN_TEST_REFRESH_FIXTURES=1 + OPENROUTER_API_KEY to capture one",
            path.display()
        );
    }

    let fixture = fs::read_to_string(&path).expect("fixture file should be readable");

    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .body(&fixture);
    });

    let provider = build_provider(&server.base_url(), "test-key-not-real");
    let response = provider
        .chat_with_tools(&[ChatMessage::user(REFRESH_PROMPT)], None, None)
        .await
        .expect("provider should parse fixture response");

    mock.assert();

    // Shape assertions — pin every field the orchestrator actually reads.
    // Do NOT over-assert on optional fields OpenRouter may omit (refusal,
    // native_finish_reason, etc.) — those are not part of our contract.

    let text = response
        .text()
        .expect("choices[0].message.content must be present");
    assert!(
        !text.is_empty(),
        "response content must be non-empty, got: {text:?}"
    );

    let finish = response
        .finish_reason()
        .expect("choices[0].finish_reason must be present");
    assert!(
        !finish.is_empty(),
        "finish_reason must be non-empty, got: {finish:?}"
    );

    let usage = response.usage().expect("usage object must be present");
    assert!(
        usage.prompt_tokens > 0,
        "usage.prompt_tokens must be > 0, got: {}",
        usage.prompt_tokens
    );
    assert!(
        usage.completion_tokens > 0,
        "usage.completion_tokens must be > 0, got: {}",
        usage.completion_tokens
    );
    assert_eq!(
        usage.total_tokens,
        usage.prompt_tokens + usage.completion_tokens,
        "usage.total_tokens should equal prompt + completion (pin the invariant \
         orchestrator budgeting relies on)"
    );

    // No tool calls expected for a plain chat turn.
    assert!(
        response.tool_calls().is_none(),
        "plain chat completion should not emit tool_calls, got: {:?}",
        response.tool_calls()
    );
}

/// Informational freshness check — warns (does not fail) if the fixture
/// is older than `FRESHNESS_WARN_AFTER`.  Operators should refresh
/// quarterly, or when the shape test fails due to drift.
#[test]
fn openrouter_fixture_freshness_warning() {
    let path = fixture_path();
    if !path.exists() {
        // Absence is covered by the shape test's panic message; not our job here.
        return;
    }
    let metadata = match fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[WARN] could not stat fixture {}: {e}", path.display());
            return;
        }
    };
    let modified = match metadata.modified() {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "[WARN] platform does not support mtime on {}: {e}",
                path.display()
            );
            return;
        }
    };
    let age = match SystemTime::now().duration_since(modified) {
        Ok(a) => a,
        Err(_) => Duration::ZERO, // fixture timestamp is in the future — treat as fresh
    };
    if age > FRESHNESS_WARN_AFTER {
        let days = age.as_secs() / 86_400;
        eprintln!(
            "[WARN] OpenRouter contract fixture {} is {days} days old \
             (threshold: {} days).  Refresh with: \
             OPENROUTER_API_KEY=<key> CAIRN_TEST_REFRESH_FIXTURES=1 \
             cargo test -p cairn-providers --test contract_openrouter",
            path.display(),
            FRESHNESS_WARN_AFTER.as_secs() / 86_400,
        );
    }
}

/// Sanity check — confirm the fixture is syntactically valid JSON so a
/// bad commit doesn't slip through review.  Cheap and catches the common
/// failure mode (hand-edit that breaks a comma).
#[test]
fn openrouter_fixture_is_valid_json() {
    let path = fixture_path();
    if !path.exists() {
        return; // covered by the shape test
    }
    let body = fs::read_to_string(&path).expect("fixture readable");
    let _: serde_json::Value = serde_json::from_str(&body).expect("fixture must be valid JSON");
}

/// Regression guard — the fixture must not contain anything that looks
/// like a real API key.  Cheap safety net in case a future refresher
/// accidentally captures auth-echoing error payloads.
#[test]
fn openrouter_fixture_contains_no_secrets() {
    let path = fixture_path();
    if !path.exists() {
        return;
    }
    let body = fs::read_to_string(&path).expect("fixture readable");
    // OpenRouter keys start with `sk-or-` and are long.  Be permissive —
    // block anything that could plausibly be a bearer token.
    for needle in &["sk-or-", "sk-proj-", "Bearer ", "bearer "] {
        assert!(
            !body.contains(needle),
            "fixture {} contains suspected secret marker {:?}",
            path.display(),
            needle
        );
    }
    // Also block the env var name appearing with a value.
    assert!(
        !body.contains("OPENROUTER_API_KEY"),
        "fixture {} references OPENROUTER_API_KEY — sanitize before committing",
        path.display()
    );
    // Silence unused-import complaints in pathological builds.
    let _ = Path::new(".");
}
