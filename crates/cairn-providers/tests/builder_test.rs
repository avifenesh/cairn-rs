use cairn_providers::{
    Backend, ChatMessage, ProviderBuilder, StructuredOutput, Tool, ToolChoice,
    error::ProviderError, wire::openai_compat::ProviderConfig,
};
use httpmock::prelude::*;
use serde_json::json;

#[derive(Debug, Clone)]
struct BackendCase {
    backend: Backend,
    path: &'static str,
    model: &'static str,
}

fn chat_response_body() -> String {
    serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "builder ok"
            }
        }],
        "usage": {
            "prompt_tokens": 3,
            "completion_tokens": 2,
            "total_tokens": 5
        }
    })
    .to_string()
}

#[test]
fn backend_configs_match_expected_presets() {
    let cases = [
        (Backend::OpenAI, ProviderConfig::OPENAI),
        (Backend::Anthropic, ProviderConfig::ANTHROPIC),
        (Backend::Ollama, ProviderConfig::OLLAMA),
        (Backend::DeepSeek, ProviderConfig::DEEPSEEK),
        (Backend::Xai, ProviderConfig::XAI),
        (Backend::Google, ProviderConfig::GOOGLE),
        (Backend::Groq, ProviderConfig::GROQ),
        (Backend::AzureOpenAI, ProviderConfig::AZURE_OPENAI),
        (Backend::OpenRouter, ProviderConfig::OPENROUTER),
        (Backend::MiniMax, ProviderConfig::MINIMAX),
        (Backend::BedrockCompat, ProviderConfig::BEDROCK_COMPAT),
        (Backend::OpenAiCompatible, ProviderConfig::default()),
        (Backend::Bedrock, ProviderConfig::default()),
    ];

    for (backend, expected) in cases {
        let actual = backend.config();
        assert_eq!(actual.name, expected.name, "{backend} name");
        assert_eq!(
            actual.default_base_url, expected.default_base_url,
            "{backend} base url"
        );
        assert_eq!(
            actual.default_model, expected.default_model,
            "{backend} default model"
        );
        assert_eq!(
            actual.chat_endpoint, expected.chat_endpoint,
            "{backend} endpoint"
        );
        assert_eq!(
            actual.supports_reasoning_effort, expected.supports_reasoning_effort,
            "{backend} reasoning flag"
        );
        assert_eq!(
            actual.supports_structured_output, expected.supports_structured_output,
            "{backend} structured output flag"
        );
        assert_eq!(
            actual.supports_parallel_tool_calls, expected.supports_parallel_tool_calls,
            "{backend} parallel tools flag"
        );
        assert_eq!(
            actual.supports_stream_options, expected.supports_stream_options,
            "{backend} stream options flag"
        );
    }
}

#[tokio::test]
async fn provider_builder_uses_backend_defaults_for_openai_style_backends() {
    let cases = [
        BackendCase {
            backend: Backend::OpenAI,
            path: "/chat/completions",
            model: "gpt-4.1-nano",
        },
        BackendCase {
            backend: Backend::Anthropic,
            path: "/messages",
            model: "claude-sonnet-4-6",
        },
        BackendCase {
            backend: Backend::Ollama,
            path: "/chat/completions",
            model: "llama3.2:3b",
        },
        BackendCase {
            backend: Backend::DeepSeek,
            path: "/chat/completions",
            model: "deepseek-chat",
        },
        BackendCase {
            backend: Backend::Xai,
            path: "/chat/completions",
            model: "grok-3-mini",
        },
        BackendCase {
            backend: Backend::Google,
            path: "/chat/completions",
            model: "gemini-2.5-flash",
        },
        BackendCase {
            backend: Backend::Groq,
            path: "/chat/completions",
            model: "llama-3.3-70b-versatile",
        },
        BackendCase {
            backend: Backend::AzureOpenAI,
            path: "/chat/completions",
            model: "gpt-4.1",
        },
        BackendCase {
            backend: Backend::OpenRouter,
            path: "/chat/completions",
            model: "openrouter/auto",
        },
        BackendCase {
            backend: Backend::MiniMax,
            path: "/chat/completions",
            model: "MiniMax-M1",
        },
        BackendCase {
            backend: Backend::BedrockCompat,
            path: "/v1/chat/completions",
            model: "us.anthropic.claude-sonnet-4-6-v1",
        },
        BackendCase {
            backend: Backend::OpenAiCompatible,
            path: "/chat/completions",
            model: "default",
        },
        BackendCase {
            backend: Backend::Zai,
            path: "/chat/completions",
            model: "glm-4.7",
        },
        BackendCase {
            backend: Backend::ZaiCoding,
            path: "/chat/completions",
            model: "glm-4.7",
        },
    ];

    for case in cases {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path(case.path)
                .header("authorization", "Bearer test-key")
                .json_body_includes(format!(r#"{{"model":"{}"}}"#, case.model))
                .json_body_includes(r#"{"stream":false}"#);
            then.status(200)
                .header("content-type", "application/json")
                .body(chat_response_body());
        });

        let provider = ProviderBuilder::new(case.backend)
            .api_key("test-key")
            .base_url(server.base_url())
            .build_chat()
            .expect("provider should build");

        let response = provider
            .chat_with_tools(&[ChatMessage::user("hello from builder")], None, None)
            .await
            .expect("chat should succeed");

        assert_eq!(response.text().as_deref(), Some("builder ok"));
        mock.assert();
    }
}

#[test]
fn provider_builder_constructs_bedrock_chat_provider() {
    let provider = ProviderBuilder::new(Backend::Bedrock)
        .api_key("bedrock-key")
        .model("anthropic.claude-3-7-sonnet")
        .region("eu-west-1")
        .build_chat();

    assert!(provider.is_ok(), "bedrock builder should succeed");
}

#[test]
fn provider_builder_rejects_invalid_base_url_without_panicking() {
    let result = ProviderBuilder::new(Backend::OpenAI)
        .api_key("test-key")
        .base_url("not a valid url")
        .build_chat();

    assert!(
        matches!(result, Err(ProviderError::InvalidRequest(message)) if message.contains("invalid base URL"))
    );
}

#[test]
fn provider_builder_requires_endpoint_for_generic_backend() {
    let result = ProviderBuilder::new(Backend::OpenAiCompatible)
        .api_key("test-key")
        .build_chat();

    assert!(
        matches!(result, Err(ProviderError::InvalidRequest(message)) if message.contains("endpoint URL required for generic backend"))
    );
}

#[tokio::test]
async fn bedrock_rejects_tools_and_structured_output_until_supported() {
    let provider = ProviderBuilder::new(Backend::Bedrock)
        .api_key("bedrock-key")
        .model("anthropic.claude-3-7-sonnet")
        .region("eu-west-1")
        .build_chat()
        .expect("bedrock builder should succeed");

    let tool = Tool {
        tool_type: "function".to_owned(),
        function: cairn_providers::chat::FunctionDef {
            name: "search".to_owned(),
            description: "search docs".to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "q": { "type": "string" }
                }
            }),
        },
    };
    let schema = StructuredOutput {
        name: "result".to_owned(),
        description: None,
        schema: Some(json!({
            "type": "object",
            "properties": {
                "answer": { "type": "string" }
            }
        })),
        strict: Some(true),
    };

    let error = provider
        .chat_with_tools(&[ChatMessage::user("hello")], Some(&[tool]), Some(schema))
        .await
        .expect_err("bedrock should reject tools/schema until implemented");

    assert!(
        matches!(error, ProviderError::Unsupported(message) if message.contains("does not support tools or structured output"))
    );
    let _ = ToolChoice::Auto;
}
