use async_trait::async_trait;
use cairn_domain::{ProjectKey, SourceId};
use serde::{Deserialize, Serialize};

/// Source-level quality summary for operator visibility (RFC 003).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceQualityRecord {
    pub source_id: SourceId,
    pub project: ProjectKey,
    pub total_chunks: u64,
    pub total_retrievals: u64,
    pub avg_relevance_score: f64,
    pub freshness_score: f64,
    pub credibility_score: f64,
    pub last_ingested_at: u64,
    #[serde(default)]
    pub avg_rating: f64,
    #[serde(default)]
    pub retrieval_count: u64,
    #[serde(default)]
    pub query_hit_rate: f64,
    #[serde(default)]
    pub error_rate: f64,
}

/// Embedding and index status for operator surfaces.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexStatus {
    pub project: ProjectKey,
    pub total_documents: u64,
    pub total_chunks: u64,
    pub pending_embeddings: u64,
    pub stale_chunks: u64,
}

/// Diagnostics service boundary for operator visibility into retrieval quality.
///
/// Per RFC 003, the product must expose: ingest status, embedding/index status,
/// source quality views, retrieval diagnostics, top-hit inspection, and
/// why-this-result explanations.
#[async_trait]
pub trait DiagnosticsService: Send + Sync {
    /// Get quality summary for a source.
    async fn source_quality(
        &self,
        source_id: &SourceId,
    ) -> Result<Option<SourceQualityRecord>, DiagnosticsError>;

    /// List source quality records for a project, ranked by relevance impact.
    async fn list_source_quality(
        &self,
        project: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<SourceQualityRecord>, DiagnosticsError>;

    /// Get overall index status for a project.
    async fn index_status(&self, project: &ProjectKey) -> Result<IndexStatus, DiagnosticsError>;
}

/// Diagnostics-specific errors.
#[derive(Debug)]
pub enum DiagnosticsError {
    StorageError(String),
    Internal(String),
}

impl std::fmt::Display for DiagnosticsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiagnosticsError::StorageError(msg) => write!(f, "storage error: {msg}"),
            DiagnosticsError::Internal(msg) => write!(f, "internal diagnostics error: {msg}"),
        }
    }
}

impl std::error::Error for DiagnosticsError {}
