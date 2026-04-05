use async_trait::async_trait;
use cairn_domain::{ChunkId, KnowledgeDocumentId, KnowledgePackId, ProjectKey, SourceId};
use serde::{Deserialize, Serialize};

/// Supported source types for v1 ingest (RFC 003).
///
/// PDF/office extraction is additive and not part of the first sellable floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    PlainText,
    Markdown,
    Html,
    StructuredJson,
    JsonStructured,
    KnowledgePack,
}

/// Ingest job status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestStatus {
    Pending,
    Parsing,
    Chunking,
    Embedding,
    Indexing,
    Completed,
    Failed,
}

/// A chunk produced by the ingest pipeline, retaining provenance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub chunk_id: ChunkId,
    pub document_id: KnowledgeDocumentId,
    pub source_id: SourceId,
    pub source_type: SourceType,
    pub project: ProjectKey,
    pub text: String,
    pub position: u32,
    pub created_at: u64,
    pub updated_at: Option<u64>,
    pub provenance_metadata: Option<serde_json::Value>,
    pub credibility_score: Option<f64>,
    pub graph_linkage: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub content_hash: Option<String>,
    /// Named entities extracted from this chunk by the entity extraction pipeline.
    ///
    /// Populated during ingest when an `EntityExtractor` is configured.
    /// Contains persons, organizations, and locations in a flat deduplicated list.
    #[serde(default)]
    pub entities: Vec<String>,
}

/// Request to ingest a document into the owned retrieval pipeline.
#[derive(Clone, Debug)]
pub struct IngestRequest {
    pub document_id: KnowledgeDocumentId,
    pub source_id: SourceId,
    pub source_type: SourceType,
    pub project: ProjectKey,
    pub content: String,
    /// Optional stable import ID for idempotent re-ingestion.
    pub import_id: Option<String>,
    /// Corpus or collection this document belongs to.
    pub corpus_id: Option<String>,
    /// Source bundle ID when ingested via a bundle import.
    pub bundle_source_id: Option<String>,
    /// Tags attached to the document for filtering.
    pub tags: Vec<String>,
}

/// Request to ingest a curated knowledge pack (RFC 013 bundle).
#[derive(Clone, Debug)]
pub struct IngestPackRequest {
    pub pack_id: KnowledgePackId,
    pub project: ProjectKey,
    /// The serialized bundle JSON. Parsed internally to extract documents.
    pub bundle_json: String,
}

/// Ingest service boundary.
///
/// Per RFC 003, ingest runs as a runtime-owned job that passes through:
/// source registration, normalization, parsing, chunking, metadata extraction,
/// deduplication, embedding generation, and index update.
///
/// Heavier ingest work may execute asynchronously but reports through
/// runtime-owned command/event surfaces.
#[async_trait]
pub trait IngestService: Send + Sync {
    /// Submit a document for ingest. Returns immediately; processing is async.
    async fn submit(&self, request: IngestRequest) -> Result<(), IngestError>;

    /// Submit a knowledge pack for ingest.
    async fn submit_pack(&self, request: IngestPackRequest) -> Result<(), IngestError>;

    /// Query the ingest status for a document.
    async fn status(
        &self,
        document_id: &KnowledgeDocumentId,
    ) -> Result<Option<IngestStatus>, IngestError>;
}

/// Ingest-specific errors.
#[derive(Debug)]
pub enum IngestError {
    UnsupportedSource(SourceType),
    ParseFailed(String),
    EmbeddingFailed(String),
    StorageError(String),
    Internal(String),
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngestError::UnsupportedSource(t) => write!(f, "unsupported source type: {t:?}"),
            IngestError::ParseFailed(msg) => write!(f, "parse failed: {msg}"),
            IngestError::EmbeddingFailed(msg) => write!(f, "embedding failed: {msg}"),
            IngestError::StorageError(msg) => write!(f, "storage error: {msg}"),
            IngestError::Internal(msg) => write!(f, "internal ingest error: {msg}"),
        }
    }
}

impl std::error::Error for IngestError {}

/// Read model for tracking document version history within a source.
///
/// Enables operators to see how a document's content has evolved across
/// re-ingestion events.
#[async_trait::async_trait]
pub trait DocumentVersionReadModel: Send + Sync {
    async fn list_versions(
        &self,
        document_id: &cairn_domain::KnowledgeDocumentId,
        limit: usize,
    ) -> Result<Vec<DocumentVersion>, IngestError>;
}

/// One version snapshot of an ingested document.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DocumentVersion {
    pub document_id: cairn_domain::KnowledgeDocumentId,
    pub version: u32,
    pub content_hash: String,
    pub ingested_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::{IngestStatus, SourceType};

    #[test]
    fn source_types_are_distinct() {
        assert_ne!(SourceType::PlainText, SourceType::Markdown);
        assert_ne!(SourceType::Html, SourceType::KnowledgePack);
    }

    #[test]
    fn ingest_status_terminal_check() {
        assert!(matches!(IngestStatus::Completed, IngestStatus::Completed));
        assert!(matches!(IngestStatus::Failed, IngestStatus::Failed));
    }
}
