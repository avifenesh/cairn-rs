//! Integration tests for the native Z.ai adapter.
//!
//! Fixtures are captured from the live coding endpoint
//! (`https://api.z.ai/api/coding/paas/v4/chat/completions`) on 2026-04-24.
//! See `docs/providers/zai.md` for the capture transcript.

use cairn_providers::{
    ChatMessage, ChatProvider,
    wire::zai::{ZaiConfig, ZaiProvider},
};
use futures::StreamExt;
use httpmock::prelude::*;

fn provider(server: &MockServer) -> ZaiProvider {
    ZaiProvider::new(
        ZaiConfig::CODING,
        "test-key",
        Some(server.base_url()),
        Some("glm-4.7".to_owned()),
        None,
        None,
        None,
    )
    .expect("provider should build")
}

#[tokio::test]
async fn zai_parses_coding_response_with_cached_tokens_and_reasoning() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .header("authorization", "Bearer test-key")
            .json_body_includes(r#"{"model":"glm-4.7"}"#)
            .json_body_includes(r#"{"thinking":{"type":"enabled"}}"#);
        then.status(200)
            .header("content-type", "application/json")
            .body(
                // Shape from the live probe (2026-04-24) — same fields, values tweaked.
                serde_json::json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "index": 0,
                        "message": {
                            "content": "hello there",
                            "reasoning_content": "User greeting; respond politely.",
                            "role": "assistant"
                        }
                    }],
                    "created": 1777040095,
                    "id": "2026042422145451ef24ed94084693",
                    "model": "glm-4.7",
                    "object": "chat.completion",
                    "usage": {
                        "completion_tokens": 5,
                        "completion_tokens_details": {"reasoning_tokens": 3},
                        "prompt_tokens": 10,
                        "prompt_tokens_details": {"cached_tokens": 4},
                        "total_tokens": 15
                    }
                })
                .to_string(),
            );
    });

    let p = provider(&server);
    let response = p
        .chat_with_tools(&[ChatMessage::user("hi")], None, None)
        .await
        .expect("chat should succeed");

    assert_eq!(response.text().as_deref(), Some("hello there"));
    assert_eq!(
        response.thinking().as_deref(),
        Some("User greeting; respond politely.")
    );
    let usage = response.usage().expect("usage should be present");
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 15);
    assert_eq!(usage.cached_tokens, Some(4));
    mock.assert();
}

#[tokio::test]
async fn zai_parses_tool_call_response_shape() {
    let server = MockServer::start();
    let mock = server.mock(|_when, then| {
        then.status(200)
            .header("content-type", "application/json")
            .body(
                // Live-probe shape with top-level `index` on tool_calls.
                serde_json::json!({
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "index": 0,
                        "message": {
                            "content": "I'll check the weather.",
                            "reasoning_content": "User wants weather.",
                            "role": "assistant",
                            "tool_calls": [{
                                "function": {
                                    "arguments": "{\"city\":\"Paris\"}",
                                    "name": "get_weather"
                                },
                                "id": "call_-7682507267639338722",
                                "index": 0,
                                "type": "function"
                            }]
                        }
                    }],
                    "model": "glm-4.7",
                    "usage": {
                        "prompt_tokens": 160,
                        "completion_tokens": 78,
                        "total_tokens": 238,
                        "prompt_tokens_details": {"cached_tokens": 0}
                    }
                })
                .to_string(),
            );
    });

    let p = provider(&server);
    let response = p
        .chat_with_tools(&[ChatMessage::user("weather Paris")], None, None)
        .await
        .expect("chat should succeed");

    let calls = response.tool_calls().expect("tool calls present");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_-7682507267639338722");
    assert_eq!(calls[0].function.name, "get_weather");
    assert_eq!(calls[0].function.arguments, "{\"city\":\"Paris\"}");

    // cached_tokens of 0 is dropped to None (not worth surfacing noise).
    let usage = response.usage().unwrap();
    assert_eq!(usage.cached_tokens, None);
    mock.assert();
}

#[tokio::test]
async fn zai_surfaces_1305_overload_as_rate_limited() {
    let server = MockServer::start();
    let _mock = server.mock(|_when, then| {
        // Z.ai sometimes returns HTTP 200 with the error envelope for
        // transient overloads (observed 2026-04-24 on streaming request).
        then.status(200)
            .header("content-type", "application/json")
            .body(
                r#"{"error":{"code":"1305","message":"The service may be temporarily overloaded, please try again later"}}"#,
            );
    });

    let p = provider(&server);
    let err = p
        .chat_with_tools(&[ChatMessage::user("hi")], None, None)
        .await
        .expect_err("1305 should surface");
    assert!(
        matches!(err, cairn_providers::ProviderError::RateLimited),
        "expected RateLimited, got {err:?}"
    );
}

#[tokio::test]
async fn zai_streaming_delivers_reasoning_content_deltas() {
    let server = MockServer::start();
    let mock = server.mock(|_when, then| {
        // SSE frames from the live probe, trimmed.
        let body = [
            r#"data: {"id":"x","created":1,"object":"chat.completion.chunk","model":"glm-4.7","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"Step 1"}}]}"#,
            r#"data: {"id":"x","created":1,"object":"chat.completion.chunk","model":"glm-4.7","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":" done"}}]}"#,
            r#"data: {"id":"x","created":1,"object":"chat.completion.chunk","model":"glm-4.7","choices":[{"index":0,"delta":{"role":"assistant","content":"hi"}}]}"#,
            r#"data: {"id":"x","created":1,"object":"chat.completion.chunk","model":"glm-4.7","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8,"prompt_tokens_details":{"cached_tokens":2}}}"#,
            "data: [DONE]",
            "",
        ]
        .join("\n\n");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(body);
    });

    let p = provider(&server);
    let mut stream = p
        .chat_stream_structured(&[ChatMessage::user("hi")], None, None)
        .await
        .expect("stream open");

    let mut reasoning = String::new();
    let mut content = String::new();
    let mut cached = None;
    while let Some(chunk) = stream.next().await {
        let sr = chunk.expect("chunk ok");
        for choice in &sr.choices {
            if let Some(ref r) = choice.delta.reasoning_content {
                reasoning.push_str(r);
            }
            if let Some(ref c) = choice.delta.content {
                content.push_str(c);
            }
        }
        if let Some(u) = sr.usage {
            cached = u.cached_tokens;
        }
    }
    assert_eq!(reasoning, "Step 1 done");
    assert_eq!(content, "hi");
    assert_eq!(cached, Some(2));
    mock.assert();
}

/// Live smoke against the coding endpoint.  Requires `ZAI_API_KEY` in env.
/// Ignored by default — run with
/// `ZAI_API_KEY=... cargo test -p cairn-providers --test wire_zai_test \
///     zai_live_smoke -- --ignored --nocapture`.
#[tokio::test]
#[ignore]
async fn zai_live_smoke() {
    let key = match std::env::var("ZAI_API_KEY") {
        Ok(k) => k,
        Err(_) => {
            eprintln!("ZAI_API_KEY unset, skipping smoke");
            return;
        }
    };
    let p = ZaiProvider::new(
        ZaiConfig::CODING,
        key,
        None,
        Some("glm-4.7".to_owned()),
        Some(32),
        None,
        Some(20),
    )
    .expect("provider builds");
    let res = p
        .chat_with_tools(&[ChatMessage::user("Reply with exactly: pong")], None, None)
        .await;
    match res {
        Ok(r) => {
            eprintln!("live smoke text={:?} usage={:?}", r.text(), r.usage());
            // Even on rate-limit we already shortcut via error; if we got Ok,
            // usage must be present.
            assert!(r.usage().is_some(), "live response should include usage");
        }
        Err(e) => {
            // Most commonly: RateLimited (overloaded 1305). Document and pass.
            eprintln!("live smoke non-fatal error: {e:?}");
        }
    }
}
