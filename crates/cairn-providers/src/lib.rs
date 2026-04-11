//! Cairn Providers — unified LLM provider abstraction.
//!
//! Provides a single trait surface (`CairnProvider`) for chat, completion,
//! embedding, and streaming across 12+ backends.  Each backend is feature-gated
//! so operators only compile what they need.
//!
//! # Provider construction
//!
//! Use [`ProviderBuilder`] to construct any backend from runtime config:
//!
//! ```ignore
//! use cairn_providers::{Backend, ProviderBuilder};
//!
//! let provider = ProviderBuilder::new(Backend::OpenAI)
//!     .api_key("sk-...")
//!     .model("gpt-4.1-nano")
//!     .build()?;
//! ```

pub mod backends;
pub mod bridge;
pub mod builder;
pub mod chat;
pub mod completion;
pub mod embedding;
pub mod error;
pub mod models;
pub mod wire;

pub use builder::{Backend, ProviderBuilder};
pub use chat::{
    ChatMessage, ChatProvider, ChatResponse, ChatRole, MessageContent, StreamChunk, StreamResponse,
    StructuredOutput, Tool, ToolChoice,
};
pub use completion::{CompletionProvider, CompletionRequest, CompletionResponse};
pub use embedding::EmbeddingProvider;
pub use error::ProviderError;
pub use wire::openai_compat;

pub use async_trait::async_trait;

// ── Core types ───────────────────────────────────────────────────────────────

/// A tool call emitted by the model.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

impl std::fmt::Display for ToolCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.function.name, self.function.arguments)
    }
}

/// The function name and serialized arguments within a [`ToolCall`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Token usage metadata returned by providers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    #[serde(alias = "input_tokens")]
    pub prompt_tokens: u32,
    #[serde(alias = "output_tokens")]
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Super-trait combining chat + completion + embedding + model listing.
pub trait CairnProvider:
    ChatProvider
    + CompletionProvider
    + EmbeddingProvider
    + models::ModelsProvider
    + Send
    + Sync
    + 'static
{
}
