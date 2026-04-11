//! Embedding provider trait.

use async_trait::async_trait;

use crate::error::ProviderError;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, input: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError>;
}
