use cairn_providers::{
    ChatMessage, ChatProvider, StreamChunk,
    wire::openai_compat::{OpenAiCompat, ProviderConfig},
};
use futures::StreamExt;
use httpmock::prelude::*;

fn provider(server: &MockServer) -> OpenAiCompat {
    OpenAiCompat::new(
        ProviderConfig::OPENAI,
        "test-key",
        Some(server.base_url()),
        Some("gpt-4.1-nano".to_owned()),
        None,
        None,
        None,
    )
    .expect("provider should build")
}

#[tokio::test]
async fn openai_compat_parses_chat_usage_and_tool_calls() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .header("authorization", "Bearer test-key")
            .json_body_includes(r#"{"model":"gpt-4.1-nano"}"#);
        then.status(200)
            .header("content-type", "application/json")
            .body(
                serde_json::json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": "Use the search tool",
                            "tool_calls": [{
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "search",
                                    "arguments": "{\"q\":\"rust\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 11,
                        "completion_tokens": 7,
                        "total_tokens": 18
                    }
                })
                .to_string(),
            );
    });

    let provider = provider(&server);
    let response = provider
        .chat_with_tools(&[ChatMessage::user("find rust docs")], None, None)
        .await
        .expect("chat should succeed");

    assert_eq!(response.text().as_deref(), Some("Use the search tool"));
    let usage = response.usage().expect("usage should be present");
    assert_eq!(usage.prompt_tokens, 11);
    assert_eq!(usage.completion_tokens, 7);
    assert_eq!(usage.total_tokens, 18);

    let tool_calls = response.tool_calls().expect("tool call should be present");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].id, "call_1");
    assert_eq!(tool_calls[0].function.name, "search");
    assert_eq!(tool_calls[0].function.arguments, "{\"q\":\"rust\"}");
    mock.assert();
}

#[tokio::test]
async fn openai_compat_ignores_malformed_stream_frames_and_keeps_valid_chunks() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .json_body_includes(r#"{"stream":true}"#);
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\r\n\r\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"broken\"}}\r\n\r\n",
                ": keep-alive comment\r\n\r\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\r\n\r\n",
                "data: [DONE]\r\n\r\n",
            ));
    });

    let provider = provider(&server);
    let chunks = provider
        .chat_stream(&[ChatMessage::user("hello")], None)
        .await
        .expect("stream should start")
        .collect::<Vec<_>>()
        .await;

    let parts: Vec<String> = chunks
        .into_iter()
        .map(|result| result.expect("chunk should parse"))
        .collect();
    assert_eq!(parts, vec!["Hello".to_owned(), " world".to_owned()]);
    mock.assert();
}

#[tokio::test]
async fn openai_compat_parses_streaming_text_frames() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .json_body_includes(r#"{"stream":true}"#);
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{}}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
                "data: [DONE]\n\n",
            ));
    });

    let provider = provider(&server);
    let chunks = provider
        .chat_stream(&[ChatMessage::user("hello")], None)
        .await
        .expect("stream should start")
        .collect::<Vec<_>>()
        .await;

    let parts: Vec<String> = chunks
        .into_iter()
        .map(|result| result.expect("chunk should parse"))
        .collect();
    assert_eq!(parts, vec!["Hello".to_owned(), " world".to_owned()]);
    mock.assert();
}

#[tokio::test]
async fn openai_compat_parses_streaming_tool_calls() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .json_body_includes(r#"{"stream":true}"#);
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(concat!(
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"search\",\"arguments\":\"{\\\"q\\\":\\\"ru\"}}]}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"st\\\"}\"}}]}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"total_tokens\":7}}\n\n",
                "data: [DONE]\n\n",
            ));
    });

    let provider = provider(&server);
    let events = provider
        .chat_stream_with_tools(&[ChatMessage::user("search rust")], None, None)
        .await
        .expect("tool stream should start")
        .collect::<Vec<_>>()
        .await;

    let events: Vec<StreamChunk> = events
        .into_iter()
        .map(|result| result.expect("stream event should parse"))
        .collect();

    assert!(events.iter().any(|event| matches!(
        event,
        StreamChunk::ToolUseStart { index: 0, id, name }
            if id == "call_1" && name == "search"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        StreamChunk::ToolUseComplete { index: 0, tool_call }
            if tool_call.id == "call_1"
                && tool_call.function.name == "search"
                && tool_call.function.arguments == "{\"q\":\"rust\"}"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        StreamChunk::Usage(usage) if usage.total_tokens == 7
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        StreamChunk::Done { stop_reason } if stop_reason == "tool_use"
    )));
    mock.assert();
}
