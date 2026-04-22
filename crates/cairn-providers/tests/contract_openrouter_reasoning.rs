//! Real-provider contract test for the **reasoning-model response shape**
//! on OpenRouter.
//!
//! This is the sibling of `contract_openrouter.rs` (happy-path chat
//! completion).  It pins a second, real production shape that the plain
//! contract test does not cover: a reasoning model that exhausts its
//! `max_tokens` budget on internal reasoning before emitting any
//! user-visible content.  The invariant shape is:
//!
//! - HTTP 200.
//! - `choices[0].message.content` is **`null`** (JSON null, not missing,
//!   not an empty string).
//! - `choices[0].finish_reason` is `"length"`.
//! - `choices[0].message.reasoning` and/or `reasoning_details` carry the
//!   hidden chain-of-thought tokens the model burned.
//! - `usage.completion_tokens_details.reasoning_tokens` approximately
//!   equals `usage.completion_tokens` (all budget went to reasoning).
//!
//! **Any caller using a reasoning model with a tight `max_tokens` will
//! see this shape.**  Cairn's parser has to handle it without panicking,
//! and the orchestrator has to be able to tell the user "the tool
//! returned no content, here is why (truncated)".
//!
//! # Parser audit finding
//!
//! `WireChatMsg::content` is already `Option<String>` (see
//! `src/wire/openai_compat.rs`), so serde deserializes JSON `null` into
//! `None` without panicking.  `ChatResponse::text()` returns
//! `Option<String>`, not a blind `unwrap`.  The parser is therefore
//! already correct for this shape — **no parser change is needed**.
//! Callers that panic on this shape do so in their own over-strict
//! `.expect("choices[0].message.content must be present")` assertions,
//! not in the parser.  This contract test locks in the correct parser
//! behavior so future refactors cannot silently regress it.
//!
//! # Refresh mode
//!
//! ```bash
//! OPENROUTER_API_KEY=<key> CAIRN_TEST_REFRESH_FIXTURES=1 \
//!     cargo test -p cairn-providers --test contract_openrouter_reasoning \
//!     -- --nocapture
//! ```
//!
//! Refresh posts a prompt that asks the model to reason about a
//! multi-step problem and caps `max_tokens` at a value small enough that
//! a reasoning model will exhaust its budget inside the hidden
//! chain-of-thought.  If the chosen model ever stops producing
//! `content: null` for this prompt, pick another `:free` reasoning model
//! from <https://openrouter.ai/models?q=free> and update `REFRESH_MODEL`.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use cairn_providers::{
    ChatMessage, ChatProvider,
    wire::openai_compat::{OpenAiCompat, ProviderConfig},
};
use httpmock::prelude::*;

/// Free-tier reasoning model used for refresh.  Selected from the
/// OpenRouter top-weekly free list with reasoning enabled.  If this is
/// deprecated or rate-limits, pick another `:free` reasoning model.
const REFRESH_MODEL: &str = "nvidia/nemotron-nano-9b-v2:free";

/// A prompt that invites multi-step reasoning.  Combined with a tight
/// `max_tokens`, reasoning models exhaust their budget on hidden CoT
/// tokens before emitting user-visible content.
const REFRESH_PROMPT: &str = "A farmer has chickens and cows. There are 30 heads and 74 legs. \
     How many chickens? Show your full step-by-step reasoning.";

/// Small enough that a reasoning model reliably hits `finish_reason: length`
/// with `content: null` — the model burns its budget inside the hidden
/// chain-of-thought before any visible content is emitted.
const REFRESH_MAX_TOKENS: u32 = 30;

/// Freshness warning threshold — same cadence as `contract_openrouter.rs`.
const FRESHNESS_WARN_AFTER: Duration = Duration::from_secs(90 * 24 * 60 * 60);

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("openrouter_reasoning_truncated.json")
}

fn build_provider(base_url: &str, api_key: &str) -> OpenAiCompat {
    OpenAiCompat::new(
        ProviderConfig::OPENROUTER,
        api_key,
        Some(base_url.to_owned()),
        Some(REFRESH_MODEL.to_owned()),
        Some(REFRESH_MAX_TOKENS),
        Some(0.0),
        Some(60),
    )
    .expect("OpenAiCompat should build from valid config")
}

/// Refresh mode — POST to the real OpenRouter API and capture the raw
/// response body byte-for-byte.
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
        .timeout(Duration::from_secs(120))
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
/// server and invoke the real provider.  The assertions pin the
/// reasoning-truncated contract:
///
/// 1. The parser **does not panic** on `content: null`.
/// 2. `text()` returns `None` (truncation detected by content-absence).
/// 3. `finish_reason()` returns `"length"` (truncation detected by
///    explicit signal).
/// 4. `thinking()` returns `Some(..)` carrying the hidden reasoning trace.
/// 5. `usage` is present and shows reasoning_tokens consumed the budget.
#[tokio::test]
async fn openrouter_reasoning_truncated_response_shape_matches_fixture() {
    let path = fixture_path();

    if env::var("CAIRN_TEST_REFRESH_FIXTURES").is_ok() {
        let api_key = env::var("OPENROUTER_API_KEY").expect(
            "refresh mode requires OPENROUTER_API_KEY — unset CAIRN_TEST_REFRESH_FIXTURES to run replay-only",
        );
        let body = refresh_fixture(&api_key).await;
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("OpenRouter response should be valid JSON");
        let pretty =
            serde_json::to_string_pretty(&parsed).expect("serde_json re-serialize should succeed");
        fs::write(&path, format!("{pretty}\n")).expect("fixture write should succeed");
        eprintln!("[REFRESHED] {}", path.display());
    }

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

    // Invariant 1: parser does not panic on `content: null`.  If the
    // fixture's content field is JSON null, this call must still return
    // Ok — the parser's `Option<String>` deserialization handles null
    // gracefully.
    let response = provider
        .chat_with_tools(&[ChatMessage::user(REFRESH_PROMPT)], None, None)
        .await
        .expect("provider must parse reasoning-truncated response without error");

    mock.assert();

    // Invariant 2: `text()` returns None when content is null.
    // Orchestrator callers that unwrap this would blow up — they must
    // handle the None case explicitly (this is the signal that the tool
    // returned no content).
    assert!(
        response.text().is_none(),
        "reasoning-truncated response must expose content as None, got: {:?}",
        response.text()
    );

    // Invariant 3: `finish_reason` is "length" — the explicit truncation
    // signal the orchestrator checks to differentiate "model chose to
    // emit nothing" from "budget exhausted".
    let finish = response
        .finish_reason()
        .expect("finish_reason must be present on a reasoning-truncated response");
    assert_eq!(
        finish, "length",
        "reasoning-truncated response must carry finish_reason=\"length\", got: {finish:?}"
    );

    // Invariant 4: reasoning trace is surfaced via `thinking()` so the
    // orchestrator can report *why* content was empty.  The parser reads
    // this from `message.reasoning` (with alias) — if the provider ever
    // drops the field, the test will fail and we'll know to adjust.
    let thinking = response
        .thinking()
        .expect("reasoning trace must be surfaced via thinking()");
    assert!(
        !thinking.is_empty(),
        "reasoning trace must be non-empty, got: {thinking:?}"
    );

    // Invariant 5: usage is present and reasoning_tokens accounts for
    // the bulk of the budget.  We only pin the structural invariants
    // (prompt_tokens > 0, total = prompt + completion) since per-model
    // token counts vary run-to-run.
    let usage = response
        .usage()
        .expect("usage must be present on a reasoning-truncated response");
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
        "usage.total_tokens should equal prompt + completion"
    );

    // No tool calls expected on a plain reasoning turn.
    assert!(
        response.tool_calls().is_none(),
        "reasoning-truncated response should not emit tool_calls, got: {:?}",
        response.tool_calls()
    );

    // Belt-and-braces: verify the raw fixture really does have
    // `content: null` + `finish_reason: "length"` — so a future refresh
    // that accidentally captures a non-truncated response doesn't
    // silently weaken the contract.
    let raw: serde_json::Value =
        serde_json::from_str(&fixture).expect("fixture must be valid JSON");
    let choice = &raw["choices"][0];
    assert!(
        choice["message"]["content"].is_null(),
        "fixture's choices[0].message.content must be JSON null — \
         refresh captured the wrong shape, got: {:?}",
        choice["message"]["content"]
    );
    assert_eq!(
        choice["finish_reason"].as_str(),
        Some("length"),
        "fixture's choices[0].finish_reason must be \"length\" — \
         refresh captured the wrong shape"
    );
}

/// Freshness warning — identical cadence to `contract_openrouter.rs`.
#[test]
fn openrouter_reasoning_fixture_freshness_warning() {
    let path = fixture_path();
    if !path.exists() {
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
        Err(_) => Duration::ZERO,
    };
    if age > FRESHNESS_WARN_AFTER {
        let days = age.as_secs() / 86_400;
        eprintln!(
            "[WARN] OpenRouter reasoning contract fixture {} is {days} days old \
             (threshold: {} days).  Refresh with: \
             OPENROUTER_API_KEY=<key> CAIRN_TEST_REFRESH_FIXTURES=1 \
             cargo test -p cairn-providers --test contract_openrouter_reasoning",
            path.display(),
            FRESHNESS_WARN_AFTER.as_secs() / 86_400,
        );
    }
}

/// Sanity — fixture is valid JSON.
#[test]
fn openrouter_reasoning_fixture_is_valid_json() {
    let path = fixture_path();
    if !path.exists() {
        return;
    }
    let body = fs::read_to_string(&path).expect("fixture readable");
    let _: serde_json::Value = serde_json::from_str(&body).expect("fixture must be valid JSON");
}

/// Regression guard — fixture contains no secret-shaped markers.
#[test]
fn openrouter_reasoning_fixture_contains_no_secrets() {
    let path = fixture_path();
    if !path.exists() {
        return;
    }
    let body = fs::read_to_string(&path).expect("fixture readable");
    for needle in &["sk-or-", "sk-proj-", "Bearer ", "bearer "] {
        assert!(
            !body.contains(needle),
            "fixture {} contains suspected secret marker {:?}",
            path.display(),
            needle
        );
    }
    assert!(
        !body.contains("OPENROUTER_API_KEY"),
        "fixture {} references OPENROUTER_API_KEY — sanitize before committing",
        path.display()
    );
}
