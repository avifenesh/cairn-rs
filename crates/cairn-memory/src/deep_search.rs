use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde::{Deserialize, Serialize};

use crate::retrieval::{RetrievalMode, RetrievalResult};

/// Deep search request supporting multi-hop retrieval (RFC 003).
#[derive(Clone, Debug)]
pub struct DeepSearchRequest {
    pub project: ProjectKey,
    pub query_text: String,
    pub max_hops: u32,
    pub per_hop_limit: usize,
    pub mode: RetrievalMode,
}

/// Quality gate outcome for a single hop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HopOutcome {
    Sufficient,
    NeedsExpansion,
    Exhausted,
}

/// A single hop in the deep search pipeline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeepSearchHop {
    pub hop_number: u32,
    pub sub_query: String,
    pub outcome: HopOutcome,
    pub results: Vec<RetrievalResult>,
    pub latency_ms: u64,
}

/// Complete deep search response.
#[derive(Clone, Debug)]
pub struct DeepSearchResponse {
    pub hops: Vec<DeepSearchHop>,
    pub merged_results: Vec<RetrievalResult>,
    pub total_latency_ms: u64,
}

/// Deep search service boundary.
///
/// Per RFC 003, deep search is a first-class owned subsystem with:
/// query decomposition, iterative retrieval, quality gates,
/// graph expansion hooks, and synthesis inputs from owned state.
#[async_trait]
pub trait DeepSearchService: Send + Sync {
    /// Execute a multi-hop deep search.
    async fn search(
        &self,
        request: DeepSearchRequest,
    ) -> Result<DeepSearchResponse, DeepSearchError>;
}

/// Deep-search-specific errors.
#[derive(Debug)]
pub enum DeepSearchError {
    RetrievalFailed(String),
    QualityGateFailed { hop: u32, reason: String },
    Internal(String),
}

impl std::fmt::Display for DeepSearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeepSearchError::RetrievalFailed(msg) => write!(f, "retrieval failed: {msg}"),
            DeepSearchError::QualityGateFailed { hop, reason } => {
                write!(f, "quality gate failed at hop {hop}: {reason}")
            }
            DeepSearchError::Internal(msg) => write!(f, "internal deep search error: {msg}"),
        }
    }
}

impl std::error::Error for DeepSearchError {}

#[cfg(test)]
mod tests {
    use super::HopOutcome;

    #[test]
    fn hop_outcomes_are_distinct() {
        assert_ne!(HopOutcome::Sufficient, HopOutcome::NeedsExpansion);
        assert_ne!(HopOutcome::NeedsExpansion, HopOutcome::Exhausted);
    }
}
