//! In-memory implementations for testing and local-mode use.
//!
//! Provides InMemoryDocumentStore (implements DocumentStore)
//! and InMemoryRetrieval (implements RetrievalService) for
//! end-to-end retrieval flow without a database.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};

use crate::ingest::{ChunkRecord, IngestError, IngestStatus, SourceType};
use crate::pipeline::DocumentStore;
use crate::retrieval::{
    RetrievalDiagnostics, RetrievalError, RetrievalMode, RetrievalQuery, RetrievalResponse,
    RetrievalResult, RetrievalService, ScoringBreakdown,
};

/// In-memory document store for testing.
pub struct InMemoryDocumentStore {
    docs: Mutex<HashMap<String, (IngestStatus, ProjectKey, SourceType)>>,
    chunks: Mutex<Vec<ChunkRecord>>,
}

impl InMemoryDocumentStore {
    pub fn new() -> Self {
        Self {
            docs: Mutex::new(HashMap::new()),
            chunks: Mutex::new(Vec::new()),
        }
    }

    /// Get all stored chunks (for retrieval queries).
    pub fn all_chunks(&self) -> Vec<ChunkRecord> {
        self.chunks.lock().unwrap().clone()
    }
}

impl Default for InMemoryDocumentStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DocumentStore for InMemoryDocumentStore {
    async fn insert_document(
        &self,
        doc_id: &KnowledgeDocumentId,
        _source_id: &SourceId,
        source_type: SourceType,
        project: &ProjectKey,
        _title: Option<&str>,
    ) -> Result<(), IngestError> {
        self.docs.lock().unwrap().insert(
            doc_id.as_str().to_owned(),
            (IngestStatus::Pending, project.clone(), source_type),
        );
        Ok(())
    }

    async fn update_status(
        &self,
        doc_id: &KnowledgeDocumentId,
        status: IngestStatus,
    ) -> Result<(), IngestError> {
        if let Some(entry) = self.docs.lock().unwrap().get_mut(doc_id.as_str()) {
            entry.0 = status;
        }
        Ok(())
    }

    async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> Result<(), IngestError> {
        self.chunks.lock().unwrap().extend(chunks.iter().cloned());
        Ok(())
    }

    async fn get_status(
        &self,
        doc_id: &KnowledgeDocumentId,
    ) -> Result<Option<IngestStatus>, IngestError> {
        Ok(self
            .docs
            .lock()
            .unwrap()
            .get(doc_id.as_str())
            .map(|(s, _, _)| *s))
    }
}

/// In-memory retrieval service using simple substring matching.
///
/// Not production-grade — this is for testing and local dev only.
/// Uses case-insensitive substring matching for lexical search.
pub struct InMemoryRetrieval {
    store: std::sync::Arc<InMemoryDocumentStore>,
}

impl InMemoryRetrieval {
    pub fn new(store: std::sync::Arc<InMemoryDocumentStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl RetrievalService for InMemoryRetrieval {
    async fn query(&self, query: RetrievalQuery) -> Result<RetrievalResponse, RetrievalError> {
        // Mode honesty: VectorOnly is not supported in the in-memory backend.
        // Hybrid explicitly falls back to lexical-only and reports it in diagnostics.
        let effective_mode = match query.mode {
            RetrievalMode::VectorOnly => {
                return Err(RetrievalError::Internal(
                    "VectorOnly mode is not supported in the in-memory backend. \
                     Use LexicalOnly or Hybrid (which falls back to lexical)."
                        .to_owned(),
                ));
            }
            RetrievalMode::Hybrid => RetrievalMode::LexicalOnly, // explicit fallback
            other => other,
        };

        let start = std::time::Instant::now();
        let chunks = self.store.all_chunks();
        let query_lower = query.query_text.to_lowercase();

        let words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(ChunkRecord, f64)> = chunks
            .into_iter()
            .filter(|c| c.project == query.project)
            .filter_map(|c| {
                let text_lower = c.text.to_lowercase();
                let matches = words.iter().filter(|w| text_lower.contains(*w)).count();
                if matches == 0 {
                    return None;
                }
                let score = matches as f64 / words.len().max(1) as f64;
                Some((c, score))
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(query.limit);

        let results: Vec<RetrievalResult> = scored
            .into_iter()
            .map(|(chunk, score)| RetrievalResult {
                chunk,
                score,
                breakdown: ScoringBreakdown {
                    lexical_relevance: score,
                    ..ScoringBreakdown::default()
                },
            })
            .collect();

        let elapsed = start.elapsed().as_millis() as u64;

        Ok(RetrievalResponse {
            results: results.clone(),
            diagnostics: RetrievalDiagnostics {
                mode_used: effective_mode,
                reranker_used: query.reranker,
                candidates_generated: results.len(),
                results_returned: results.len(),
                latency_ms: elapsed,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{IngestRequest, IngestService};
    use crate::pipeline::{IngestPipeline, ParagraphChunker};
    use crate::retrieval::{RerankerStrategy, RetrievalMode};
    use std::sync::Arc;

    /// End-to-end test: ingest plain text documents, then query retrieval.
    #[tokio::test]
    async fn end_to_end_ingest_and_retrieve() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let chunker = ParagraphChunker {
            max_chunk_size: 200,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker);
        let retrieval = InMemoryRetrieval::new(store.clone());

        // Ingest a plain text document.
        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_rust"),
                source_id: SourceId::new("src_docs"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content:
                    "Rust is a systems programming language focused on safety and performance.\n\n\
                           The borrow checker ensures memory safety without garbage collection.\n\n\
                           Cargo is the Rust package manager and build tool."
                        .to_owned(),
            })
            .await
            .unwrap();

        // Ingest a markdown document.
        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_python"),
                source_id: SourceId::new("src_docs"),
                source_type: SourceType::Markdown,
                project: ProjectKey::new("t", "w", "p"),
                content: "# Python\n\nPython is a high-level programming language.\n\n\
                           It has dynamic typing and garbage collection."
                    .to_owned(),
            })
            .await
            .unwrap();

        // Query for "borrow checker memory safety".
        let response = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "borrow checker memory safety".to_owned(),
                mode: RetrievalMode::LexicalOnly,
                reranker: RerankerStrategy::None,
                limit: 5,
                metadata_filters: vec![],
            })
            .await
            .unwrap();

        // Should find the borrow checker chunk.
        assert!(!response.results.is_empty());
        assert!(response.results[0].chunk.text.contains("borrow checker"));
        assert!(response.results[0].score > 0.0);

        // Diagnostics should be populated.
        assert_eq!(response.diagnostics.mode_used, RetrievalMode::LexicalOnly);
        assert!(response.diagnostics.candidates_generated > 0);

        // Query for "garbage collection" — should match both Rust and Python.
        let gc_response = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "garbage collection".to_owned(),
                mode: RetrievalMode::LexicalOnly,
                reranker: RerankerStrategy::None,
                limit: 10,
                metadata_filters: vec![],
            })
            .await
            .unwrap();

        assert!(gc_response.results.len() >= 2);

        // Query with wrong project — should return nothing.
        let empty = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("other", "w", "p"),
                query_text: "rust".to_owned(),
                mode: RetrievalMode::LexicalOnly,
                reranker: RerankerStrategy::None,
                limit: 5,
                metadata_filters: vec![],
            })
            .await
            .unwrap();

        assert!(empty.results.is_empty());
    }

    /// Verify all v1 supported document types can be ingested.
    #[tokio::test]
    async fn supports_all_v1_document_types() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let chunker = ParagraphChunker::default();
        let pipeline = IngestPipeline::new(store.clone(), chunker);

        let types = [
            (SourceType::PlainText, "Plain text content."),
            (SourceType::Markdown, "# Heading\n\nMarkdown content."),
            (SourceType::Html, "<p>HTML content.</p>"),
            (SourceType::StructuredJson, r#"{"key": "JSON content"}"#),
        ];

        for (i, (source_type, content)) in types.iter().enumerate() {
            pipeline
                .submit(IngestRequest {
                    document_id: KnowledgeDocumentId::new(format!("doc_{i}")),
                    source_id: SourceId::new("src"),
                    source_type: *source_type,
                    project: ProjectKey::new("t", "w", "p"),
                    content: content.to_string(),
                })
                .await
                .unwrap();

            let status = pipeline
                .status(&KnowledgeDocumentId::new(format!("doc_{i}")))
                .await
                .unwrap();
            assert_eq!(status, Some(IngestStatus::Completed));
        }

        assert!(store.all_chunks().len() >= 4);
    }

    /// Mode contract: VectorOnly is rejected with explicit error.
    #[tokio::test]
    async fn vector_only_mode_is_rejected() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let retrieval = InMemoryRetrieval::new(store);

        let result = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "test".to_owned(),
                mode: RetrievalMode::VectorOnly,
                reranker: RerankerStrategy::None,
                limit: 5,
                metadata_filters: vec![],
            })
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("VectorOnly"),
            "error must name the unsupported mode"
        );
    }

    /// Mode contract: Hybrid falls back to LexicalOnly and reports it in diagnostics.
    #[tokio::test]
    async fn hybrid_mode_reports_lexical_fallback() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let chunker = ParagraphChunker::default();
        let pipeline = IngestPipeline::new(store.clone(), chunker);

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_mode"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Hybrid mode fallback test content.".to_owned(),
            })
            .await
            .unwrap();

        let retrieval = InMemoryRetrieval::new(store);

        let response = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "hybrid fallback".to_owned(),
                mode: RetrievalMode::Hybrid,
                reranker: RerankerStrategy::None,
                limit: 5,
                metadata_filters: vec![],
            })
            .await
            .unwrap();

        assert_eq!(
            response.diagnostics.mode_used,
            RetrievalMode::LexicalOnly,
            "Hybrid must report LexicalOnly in diagnostics, not Hybrid"
        );
    }
}
