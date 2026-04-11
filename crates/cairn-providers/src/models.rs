//! Model listing trait for providers that expose their model catalog.

use async_trait::async_trait;

use crate::error::ProviderError;

/// A single model in a provider's catalog.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

#[async_trait]
pub trait ModelsProvider: Send + Sync {
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(vec![])
    }
}
