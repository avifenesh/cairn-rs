//! Text completion provider trait.

use async_trait::async_trait;

use crate::{ToolCall, chat::ChatResponse, error::ProviderError};

/// A request for text completion.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prompt: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

impl CompletionRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            max_tokens: None,
            temperature: None,
        }
    }
}

/// Generated text from a completion request.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub text: String,
}

impl ChatResponse for CompletionResponse {
    fn text(&self) -> Option<String> {
        Some(self.text.clone())
    }
    fn tool_calls(&self) -> Option<Vec<ToolCall>> {
        None
    }
}

impl std::fmt::Display for CompletionResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

#[async_trait]
pub trait CompletionProvider: Send + Sync {
    async fn complete(
        &self,
        req: &CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError>;
}
