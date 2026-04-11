use cairn_domain::providers::{
    GenerationProvider, ProviderAdapterError, ProviderBindingSettings,
};
use cairn_providers::wire::openai_compat::{OpenAiCompat, ProviderConfig};
use httpmock::prelude::*;
use serde_json::json;

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
}

#[tokio::test]
async fn generation_bridge_maps_text_usage_and_tool_calls() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .json_body_includes(r#"{"model":"gpt-4.1-nano"}"#);
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": "bridge reply",
                            "tool_calls": [{
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "search",
                                    "arguments": "{\"q\":\"bridge\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 9,
                        "completion_tokens": 4,
                        "total_tokens": 13
                    }
                })
                .to_string(),
            );
    });

    let provider = provider(&server);
    let response = GenerationProvider::generate(
        &provider,
        "caller-model-is-ignored-for-now",
        vec![json!({ "role": "user", "content": "hello bridge" })],
        &ProviderBindingSettings::default(),
    )
    .await
    .expect("generation bridge should succeed");

    assert_eq!(response.text, "bridge reply");
    assert_eq!(response.input_tokens, Some(9));
    assert_eq!(response.output_tokens, Some(4));
    assert_eq!(response.model_id, "gpt-4.1-nano");
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0]["id"], "call_1");
    assert_eq!(response.tool_calls[0]["function"]["name"], "search");
    assert_eq!(
        response.tool_calls[0]["function"]["arguments"],
        "{\"q\":\"bridge\"}"
    );
    mock.assert();
}

#[tokio::test]
async fn generation_bridge_maps_rate_limit_errors() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(429)
            .header("content-type", "application/json")
            .body("{\"error\":\"slow down\"}");
    });

    let provider = provider(&server);
    let error = GenerationProvider::generate(
        &provider,
        "ignored",
        vec![json!({ "role": "user", "content": "hello bridge" })],
        &ProviderBindingSettings::default(),
    )
    .await
    .expect_err("rate limiting should surface as an adapter error");

    assert!(matches!(error, ProviderAdapterError::RateLimited));
    mock.assert();
}
