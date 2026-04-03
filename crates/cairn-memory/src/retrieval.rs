use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde::{Deserialize, Serialize};

use crate::ingest::ChunkRecord;

/// Retrieval mode selection (RFC 003).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode {
    LexicalOnly,
    VectorOnly,
    Hybrid,
}

/// Reranker strategy applied after candidate generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RerankerStrategy {
    None,
    Mmr,
    ProviderReranker,
}

/// Canonical scoring dimensions (RFC 003).
///
/// All dimensions are fixed by the product contract and must be present
/// in every compliant retrieval implementation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScoringBreakdown {
    pub semantic_relevance: f64,
    pub lexical_relevance: f64,
    pub freshness: f64,
    pub staleness_penalty: f64,
    pub source_credibility: f64,
    pub corroboration: f64,
    pub graph_proximity: f64,
    pub recency_of_use: Option<f64>,
}

/// A scored retrieval result with inspectable scoring.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetrievalResult {
    pub chunk: ChunkRecord,
    pub score: f64,
    pub breakdown: ScoringBreakdown,
}

/// Retrieval query with operator-tunable parameters.
#[derive(Clone, Debug)]
pub struct RetrievalQuery {
    pub project: ProjectKey,
    pub query_text: String,
    pub mode: RetrievalMode,
    pub reranker: RerankerStrategy,
    pub limit: usize,
    pub metadata_filters: Vec<MetadataFilter>,
}

/// Simple metadata filter for retrieval queries.
#[derive(Clone, Debug)]
pub struct MetadataFilter {
    pub key: String,
    pub value: String,
}

/// Retrieval diagnostics for a completed query (RFC 003 requirement).
///
/// For every retrieval request, the product must expose: retrieval mode,
/// candidate-generation stages, scoring dimensions that contributed,
/// effective scoring policy, and reranker path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetrievalDiagnostics {
    pub mode_used: RetrievalMode,
    pub reranker_used: RerankerStrategy,
    pub candidates_generated: usize,
    pub results_returned: usize,
    pub latency_ms: u64,
}

/// A retrieval response including results and diagnostics.
#[derive(Clone, Debug)]
pub struct RetrievalResponse {
    pub results: Vec<RetrievalResult>,
    pub diagnostics: RetrievalDiagnostics,
}

/// Retrieval service boundary.
///
/// Per RFC 003, retrieval runs in-process with the main runtime/API and
/// supports lexical, vector, and hybrid modes with metadata filtering,
/// reranking, and inspectable scoring.
#[async_trait]
pub trait RetrievalService: Send + Sync {
    /// Execute a retrieval query and return scored results with diagnostics.
    async fn query(&self, query: RetrievalQuery) -> Result<RetrievalResponse, RetrievalError>;
}

/// Retrieval-specific errors.
#[derive(Debug)]
pub enum RetrievalError {
    EmbeddingFailed(String),
    StorageError(String),
    Internal(String),
}

impl std::fmt::Display for RetrievalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RetrievalError::EmbeddingFailed(msg) => write!(f, "embedding failed: {msg}"),
            RetrievalError::StorageError(msg) => write!(f, "storage error: {msg}"),
            RetrievalError::Internal(msg) => write!(f, "internal retrieval error: {msg}"),
        }
    }
}

impl std::error::Error for RetrievalError {}

#[cfg(test)]
mod tests {
    use super::{RerankerStrategy, RetrievalMode, ScoringBreakdown};

    #[test]
    fn retrieval_modes_are_distinct() {
        assert_ne!(RetrievalMode::LexicalOnly, RetrievalMode::VectorOnly);
        assert_ne!(RetrievalMode::Hybrid, RetrievalMode::LexicalOnly);
    }

    #[test]
    fn default_scoring_breakdown_is_zero() {
        let b = ScoringBreakdown::default();
        assert_eq!(b.semantic_relevance, 0.0);
        assert_eq!(b.recency_of_use, None);
    }

    #[test]
    fn reranker_strategies_are_distinct() {
        assert_ne!(RerankerStrategy::None, RerankerStrategy::Mmr);
    }
}
