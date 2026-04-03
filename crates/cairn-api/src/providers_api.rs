use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde::{Deserialize, Serialize};

/// Provider health status for operator visibility.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderHealthSummary {
    pub provider_id: String,
    pub status: String,
    pub recent_failures: u32,
    pub avg_latency_ms: Option<u64>,
}

/// Provider settings and health endpoints per RFC 010.
#[async_trait]
pub trait ProviderEndpoints: Send + Sync {
    type Error;
    async fn list_provider_health(
        &self,
        project: &ProjectKey,
    ) -> Result<Vec<ProviderHealthSummary>, Self::Error>;
}
