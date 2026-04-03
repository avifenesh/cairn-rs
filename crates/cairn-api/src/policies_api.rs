use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde::{Deserialize, Serialize};

/// Policy decision record for operator inspection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyDecisionSummary {
    pub action: String,
    pub decision: String,
    pub reason: Option<String>,
    pub scope: String,
    pub created_at: u64,
}

/// Policy inspection endpoints per RFC 010.
#[async_trait]
pub trait PolicyEndpoints: Send + Sync {
    type Error;
    async fn list_recent_decisions(
        &self,
        project: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<PolicyDecisionSummary>, Self::Error>;
}
