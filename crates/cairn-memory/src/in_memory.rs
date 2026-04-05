//! In-memory implementations for testing and local-mode use.
//!
//! Provides InMemoryDocumentStore (implements DocumentStore)
//! and InMemoryRetrieval (implements RetrievalService) for
//! end-to-end retrieval flow without a database.

use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};

use crate::ingest::{ChunkRecord, IngestError, IngestStatus, SourceType};
use crate::pipeline::DocumentStore;
use crate::reranking::mmr_rerank;
use crate::retrieval::{
    self, CandidateStage, RerankerStrategy, RetrievalDiagnostics, RetrievalError, RetrievalMode,
    RetrievalQuery, RetrievalResponse, RetrievalResult, RetrievalService, ScoringBreakdown,
};

/// In-memory document store for testing.
/// A document record suitable for export operations.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ExportableDocument {
    pub document_id: KnowledgeDocumentId,
    pub source_id: SourceId,
    pub project: ProjectKey,
    pub source_type: SourceType,
    pub text: String,
    pub credibility_score: Option<f32>,
    pub provenance: Option<serde_json::Value>,
    pub tags: Vec<String>,
    pub corpus_id: Option<String>,
    pub created_at_ms: u64,
    /// Alias for `created_at_ms` used by export filters.
    #[serde(default)]
    pub created_at: u64,
    /// Document title, if available.
    #[serde(default)]
    pub title: Option<String>,
    /// Raw provenance metadata from the ingest pipeline.
    #[serde(default)]
    pub provenance_metadata: Option<serde_json::Value>,
}

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

    /// Get mutable access to stored chunks (for retroactive metadata updates).
    pub fn chunks_mut(&self) -> std::sync::MutexGuard<'_, Vec<ChunkRecord>> {
        self.chunks.lock().unwrap()
    }

    /// Return document IDs whose content hash matches the given hash.
    pub fn document_ids_by_hash(&self, content_hash: &str) -> Vec<KnowledgeDocumentId> {
        let chunks = self.chunks.lock().unwrap();
        chunks
            .iter()
            .filter(|c| c.content_hash.as_deref() == Some(content_hash))
            .map(|c| c.document_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect()
    }

    /// Return all documents as `ExportableDocument` records (for export operations).
    pub fn exportable_documents(&self) -> Vec<ExportableDocument> {
        let docs = self.docs.lock().unwrap();
        let chunks = self.chunks.lock().unwrap();
        // Build a map of document_id -> (source_id, text, tags, created_at) from chunks.
        let mut chunk_info: std::collections::HashMap<&str, (&SourceId, String, Vec<String>, u64)> =
            std::collections::HashMap::new();
        for chunk in chunks.iter() {
            chunk_info
                .entry(chunk.document_id.as_str())
                .or_insert_with(|| (&chunk.source_id, chunk.text.clone(), chunk.entities.clone(), chunk.created_at));
        }
        docs.iter()
            .map(|(id, (_, project, source_type))| {
                let (source_id, text, tags, created_at) = chunk_info
                    .get(id.as_str())
                    .map(|(s, t, tags, ts)| ((*s).clone(), t.clone(), tags.clone(), *ts))
                    .unwrap_or_else(|| (SourceId::new("unknown"), String::new(), vec![], 0));
                ExportableDocument {
                    document_id: KnowledgeDocumentId::new(id.clone()),
                    source_id,
                    project: project.clone(),
                    source_type: source_type.clone(),
                    text,
                    credibility_score: None,
                    provenance: None,
                    tags,
                    corpus_id: None,
                    created_at_ms: created_at,
                    created_at,
                    title: None,
                    provenance_metadata: None,
                }
            })
            .collect()
    }

    /// Remove a document from the store by ID.
    pub fn remove_document(&self, doc_id: &KnowledgeDocumentId) {
        self.docs.lock().unwrap().remove(doc_id.as_str());
        let mut chunks = self.chunks.lock().unwrap();
        chunks.retain(|c| &c.document_id != doc_id);
    }

    /// Get all current chunks (alias used by diagnostics and handlers).
    pub fn all_current_chunks(&self) -> Vec<ChunkRecord> {
        self.all_chunks()
    }

    /// List all known sources for a project.
    pub fn list_sources(&self, project: &cairn_domain::ProjectKey) -> Vec<SourceSummary> {
        let chunks = self.chunks.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        chunks.iter()
            .filter(|c| &c.project == project)
            .filter_map(|c| {
                if seen.insert(c.source_id.as_str().to_owned()) {
                    Some(SourceSummary {
                        source_id: c.source_id.clone(),
                        document_count: 1,
                        avg_quality_score: 0.0,
                        last_ingested_at_ms: Some(c.created_at),
                    })
                } else { None }
            })
            .collect()
    }

    /// Register a source (no-op stub; returns a SourceSummary).
    pub fn register_source(&self, _project: &cairn_domain::ProjectKey, source_id: &cairn_domain::SourceId) -> SourceSummary {
        SourceSummary {
            source_id: source_id.clone(),
            document_count: 0,
            avg_quality_score: 0.0,
            last_ingested_at_ms: None,
        }
    }

    /// Deactivate a source by removing all its chunks.
    pub fn deactivate_source(&self, source_id: &cairn_domain::SourceId) -> bool {
        let mut chunks = self.chunks.lock().unwrap();
        let before = chunks.len();
        chunks.retain(|c| &c.source_id != source_id);
        chunks.len() < before
    }

    /// Check if a source is active (has any chunks).
    pub fn is_source_active(&self, source_id: &cairn_domain::SourceId) -> bool {
        let chunks = self.chunks.lock().unwrap();
        chunks.iter().any(|c| &c.source_id == source_id)
    }

    /// Get a refresh schedule for a source (always None in stub).
    pub fn get_refresh_schedule(&self, _source_id: &cairn_domain::SourceId) -> Option<RefreshSchedule> {
        None
    }

    /// Create a refresh schedule for a source.
    pub fn create_refresh_schedule(
        &self,
        source_id: &cairn_domain::SourceId,
        _project: &cairn_domain::ProjectKey,
        interval_ms: u64,
        refresh_url: Option<String>,
    ) -> RefreshSchedule {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        RefreshSchedule {
            schedule_id: format!("sched_{}", now),
            source_id: source_id.clone(),
            interval_ms,
            last_refresh_ms: None,
            enabled: true,
            refresh_url,
        }
    }

    /// List all due refresh schedules (always empty in stub).
    pub fn list_due_schedules(&self, _now_ms: u64) -> Vec<RefreshSchedule> {
        vec![]
    }

    /// Update the last refresh timestamp for a schedule (no-op in stub).
    pub fn update_last_refresh_ms(&self, _schedule_id: &str, _now_ms: u64) {}
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

    async fn chunk_hashes_for_project(
        &self,
        project: &ProjectKey,
    ) -> Result<HashSet<String>, IngestError> {
        let chunks = self.chunks.lock().unwrap();
        let hashes = chunks
            .iter()
            .filter(|c| c.project == *project)
            .filter_map(|c| c.content_hash.clone())
            .collect();
        Ok(hashes)
    }
}

#[async_trait]
impl crate::ingest::DocumentVersionReadModel for InMemoryDocumentStore {
    async fn list_versions(
        &self,
        _document_id: &KnowledgeDocumentId,
        _limit: usize,
    ) -> Result<Vec<crate::ingest::DocumentVersion>, IngestError> {
        Ok(vec![])
    }
}

/// In-memory retrieval service using simple substring matching.
///
/// Not production-grade — this is for testing and local dev only.
/// Uses case-insensitive substring matching for lexical search.
pub struct InMemoryRetrieval {
    store: std::sync::Arc<InMemoryDocumentStore>,
    graph: Option<std::sync::Arc<cairn_graph::in_memory::InMemoryGraphStore>>,
}

impl InMemoryRetrieval {
    pub fn new(store: std::sync::Arc<InMemoryDocumentStore>) -> Self {
        Self { store, graph: None }
    }

    /// Create with a diagnostics adapter (stub — diagnostics adapter is ignored in in-memory backend).
    pub fn with_diagnostics(
        store: std::sync::Arc<InMemoryDocumentStore>,
        _diagnostics: std::sync::Arc<dyn crate::diagnostics::DiagnosticsService>,
    ) -> Self {
        Self { store, graph: None }
    }

    /// Attach a graph store for graph-proximity scoring.
    pub fn with_graph(
        mut self,
        graph: std::sync::Arc<cairn_graph::in_memory::InMemoryGraphStore>,
    ) -> Self {
        self.graph = Some(graph);
        self
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
            .filter(|c| {
                query.metadata_filters.iter().all(|f| {
                    c.provenance_metadata
                        .as_ref()
                        .is_some_and(|m| {
                            // Direct key match: check scalar string value.
                            if let Some(v) = m.get(&f.key).and_then(|v| v.as_str()) {
                                return v == f.value;
                            }
                            // Tag filter: "tag" checks membership in "tags" array.
                            if f.key == "tag" {
                                if let Some(arr) = m.get("tags").and_then(|v| v.as_array()) {
                                    return arr.iter().any(|v| v.as_str() == Some(&f.value));
                                }
                            }
                            false
                        })
                })
            })
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

        // For MMR, keep a larger candidate pool so diversity selection has room.
        let candidate_limit = if query.reranker == RerankerStrategy::Mmr {
            (query.limit * 3).min(scored.len())
        } else {
            query.limit
        };
        scored.truncate(candidate_limit);

        let policy = query.scoring_policy.as_ref().cloned().unwrap_or_default();
        let now = retrieval::now_ms();

        let mut scoring_dims: Vec<String> = Vec::new();

        let mut results: Vec<RetrievalResult> = scored
            .into_iter()
            .map(|(chunk, lexical_score)| {
                let fresh = retrieval::freshness_score(
                    chunk.created_at,
                    now,
                    policy.freshness_decay_days,
                );
                let stale = retrieval::staleness_penalty(
                    chunk.updated_at,
                    chunk.created_at,
                    now,
                    policy.staleness_threshold_days,
                );

                let credibility = chunk.credibility_score
                    .map(|s| s.clamp(0.0, 1.0))
                    .unwrap_or(0.0);

                let breakdown = ScoringBreakdown {
                    lexical_relevance: lexical_score,
                    freshness: fresh,
                    staleness_penalty: stale,
                    // RFC 003: source_credibility populated from the chunk record so
                    // operator feedback (which updates chunk.credibility_score) is
                    // reflected in subsequent retrieval scores.
                    source_credibility: credibility,
                    corroboration: 0.0,
                    graph_proximity: 0.0,
                    recency_of_use: Some(0.0),
                    ..ScoringBreakdown::default()
                };

                let final_score = retrieval::compute_final_score(&breakdown, &policy.weights);

                RetrievalResult {
                    chunk,
                    score: final_score,
                    breakdown,
                }
            })
            .collect();

        // Apply graph proximity: for each result, count how many OTHER result documents
        // are graph neighbors of this result's document_id. Normalize to [0, 1].
        if let Some(graph) = &self.graph {
            use cairn_graph::queries::{GraphQueryService, TraversalDirection};
            let result_doc_ids: std::collections::HashSet<String> = results
                .iter()
                .map(|r| r.chunk.document_id.as_str().to_owned())
                .collect();

            let total_others = result_doc_ids.len().saturating_sub(1).max(1) as f64;

            for result in &mut results {
                let doc_id = result.chunk.document_id.as_str();
                // Query both upstream and downstream neighbors.
                let downstream = graph
                    .neighbors(doc_id, None, TraversalDirection::Downstream, 50)
                    .await
                    .unwrap_or_default();
                let upstream = graph
                    .neighbors(doc_id, None, TraversalDirection::Upstream, 50)
                    .await
                    .unwrap_or_default();

                let neighbor_ids: std::collections::HashSet<String> = downstream
                    .iter()
                    .map(|(_, n)| n.node_id.clone())
                    .chain(upstream.iter().map(|(_, n)| n.node_id.clone()))
                    .collect();

                let overlap = neighbor_ids
                    .intersection(&result_doc_ids)
                    .filter(|id| id.as_str() != doc_id) // exclude self
                    .count();

                if overlap > 0 {
                    result.breakdown.graph_proximity = (overlap as f64 / total_others).min(1.0);
                    result.score = retrieval::compute_final_score(&result.breakdown, &policy.weights);
                }
            }

            // Re-sort after graph proximity update.
            results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        }

        // Track all dimensions that materially contributed across results.
        if results.iter().any(|r| r.breakdown.lexical_relevance != 0.0) {
            scoring_dims.push("lexical_relevance".to_owned());
        }
        if results.iter().any(|r| r.breakdown.semantic_relevance != 0.0) {
            scoring_dims.push("semantic_relevance".to_owned());
        }
        if results.iter().any(|r| r.breakdown.freshness != 0.0) {
            scoring_dims.push("freshness".to_owned());
        }
        if results.iter().any(|r| r.breakdown.staleness_penalty != 0.0) {
            scoring_dims.push("staleness_penalty".to_owned());
        }
        if results.iter().any(|r| r.breakdown.source_credibility != 0.0) {
            scoring_dims.push("source_credibility".to_owned());
        }
        if results.iter().any(|r| r.breakdown.corroboration != 0.0) {
            scoring_dims.push("corroboration".to_owned());
        }
        if results.iter().any(|r| r.breakdown.graph_proximity != 0.0) {
            scoring_dims.push("graph_proximity".to_owned());
        }
        if results.iter().any(|r| r.breakdown.recency_of_use.unwrap_or(0.0) != 0.0) {
            scoring_dims.push("recency_of_use".to_owned());
        }

        // Re-sort by final weighted score.
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        let candidates_generated = results.len();

        // Apply reranking if requested.
        let (results, stages) = match query.reranker {
            RerankerStrategy::Mmr => {
                let reranked = mmr_rerank(&results, query.limit, 0.5);
                (reranked, vec![CandidateStage::Lexical, CandidateStage::Reranked])
            }
            _ => {
                let mut r = results;
                r.truncate(query.limit);
                (r, vec![CandidateStage::Lexical])
            }
        };

        let elapsed = start.elapsed().as_millis() as u64;
        let results_returned = results.len();

        Ok(RetrievalResponse {
            diagnostics: RetrievalDiagnostics {
                mode_used: effective_mode,
                reranker_used: query.reranker,
                candidates_generated,
                results_returned,
                latency_ms: elapsed,
                stages_used: stages,
                scoring_dimensions_used: scoring_dims,
                effective_policy: Some(format!(
                    "freshness_decay={}d staleness_threshold={}d recency={}",
                    policy.freshness_decay_days,
                    policy.staleness_threshold_days,
                    if policy.recency_enabled { "on" } else { "off" },
                )),
            },
            results,
        })
    }
}

impl InMemoryRetrieval {
    /// Register a source in the document store.
    pub fn register_source(&self, project: &cairn_domain::ProjectKey, source_id: &cairn_domain::SourceId) -> SourceSummary {
        let _ = project;
        SourceSummary {
            source_id: source_id.clone(),
            document_count: 0,
            avg_quality_score: 0.0,
            last_ingested_at_ms: None,
        }
    }

    /// List all known sources for a project.
    pub fn list_sources(&self, project: &cairn_domain::ProjectKey) -> Vec<SourceSummary> {
        let chunks = self.store.chunks.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        chunks.iter()
            .filter(|c| &c.project == project)
            .filter_map(|c| {
                if seen.insert(c.source_id.as_str().to_owned()) {
                    Some(SourceSummary {
                        source_id: c.source_id.clone(),
                        document_count: 1,
                        avg_quality_score: 0.0,
                        last_ingested_at_ms: Some(c.created_at),
                    })
                } else { None }
            })
            .collect()
    }

    /// Check if a source is active (has any chunks).
    pub fn is_source_active(&self, source_id: &cairn_domain::SourceId) -> bool {
        let chunks = self.store.chunks.lock().unwrap();
        chunks.iter().any(|c| &c.source_id == source_id)
    }

    /// Deactivate a source (removes all its chunks).
    pub fn deactivate_source(&self, source_id: &cairn_domain::SourceId) -> bool {
        let mut chunks = self.store.chunks.lock().unwrap();
        let before = chunks.len();
        chunks.retain(|c| &c.source_id != source_id);
        chunks.len() < before
    }

    /// Get all chunks (alias for all_chunks used by diagnostics).
    pub fn all_current_chunks(&self) -> Vec<ChunkRecord> {
        self.store.all_chunks()
    }

    /// Create a refresh schedule for a source.
    pub fn create_refresh_schedule(
        &self,
        source_id: &cairn_domain::SourceId,
        _project: &cairn_domain::ProjectKey,
        interval_ms: u64,
        refresh_url: Option<String>,
    ) -> RefreshSchedule {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        RefreshSchedule {
            schedule_id: format!("sched_{}", now),
            source_id: source_id.clone(),
            interval_ms,
            last_refresh_ms: None,
            enabled: true,
            refresh_url,
        }
    }

    /// Get a refresh schedule for a source (always None in stub).
    pub fn get_refresh_schedule(&self, _source_id: &cairn_domain::SourceId) -> Option<RefreshSchedule> {
        None
    }

    /// List all due refresh schedules (always empty in stub).
    pub fn list_due_schedules(&self, _now_ms: u64) -> Vec<RefreshSchedule> {
        vec![]
    }

    /// Update the last refresh timestamp for a schedule (no-op in stub).
    pub fn update_last_refresh_ms(&self, _schedule_id: &str, _now_ms: u64) {}
}

/// RFC 003 §6: "why-this-result" explanation for a specific chunk + query pair.
#[derive(Clone, Debug)]
pub struct ResultExplanation {
    pub chunk_id: String,
    pub query_text: String,
    pub lexical_relevance: f64,
    pub freshness: f64,
    pub quality_score: f64,
    pub summary: String,
}

impl InMemoryRetrieval {
    /// Explain why a specific chunk would be returned for a query.
    ///
    /// Returns `None` if the chunk does not exist in this project.
    pub fn explain_result(
        &self,
        chunk_id: &str,
        query_text: &str,
        project: &cairn_domain::ProjectKey,
    ) -> Option<ResultExplanation> {
        let chunks = self.store.all_chunks();
        let chunk = chunks
            .iter()
            .find(|c| c.chunk_id.as_str() == chunk_id && &c.project == project)?;

        let query_lower = query_text.to_lowercase();
        let text_lower = chunk.text.to_lowercase();
        let words: Vec<&str> = query_lower.split_whitespace().collect();
        let matches = words.iter().filter(|w| text_lower.contains(*w)).count();
        let lexical_relevance = if words.is_empty() {
            0.0
        } else {
            matches as f64 / words.len() as f64
        };

        let now = retrieval::now_ms();
        let freshness = retrieval::freshness_score(chunk.created_at, now, 30.0);
        let quality_score = chunk.credibility_score.unwrap_or(1.0).clamp(0.0, 1.0);

        let summary = format!(
            "Chunk '{}' from source '{}': lexical_relevance={:.2}, freshness={:.2}, quality={:.2}",
            chunk_id,
            chunk.source_id.as_str(),
            lexical_relevance,
            freshness,
            quality_score
        );

        Some(ResultExplanation {
            chunk_id: chunk_id.to_owned(),
            query_text: query_text.to_owned(),
            lexical_relevance,
            freshness,
            quality_score,
            summary,
        })
    }
}

/// A scheduled refresh policy for a knowledge source.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RefreshSchedule {
    pub schedule_id: String,
    pub source_id: cairn_domain::SourceId,
    pub interval_ms: u64,
    pub last_refresh_ms: Option<u64>,
    pub enabled: bool,
    pub refresh_url: Option<String>,
}

/// Aggregate summary of a knowledge source (document count, quality score).
///
/// Used by operator-facing source management endpoints.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SourceSummary {
    pub source_id: cairn_domain::SourceId,
    pub document_count: u64,
    pub avg_quality_score: f32,
    pub last_ingested_at_ms: Option<u64>,
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
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
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
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
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
                scoring_policy: None,
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
                scoring_policy: None,
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
                scoring_policy: None,
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
                    import_id: None,
                    corpus_id: None,
                    bundle_source_id: None,
                    tags: vec![],
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
                scoring_policy: None,
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
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
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
                scoring_policy: None,
            })
            .await
            .unwrap();

        assert_eq!(
            response.diagnostics.mode_used,
            RetrievalMode::LexicalOnly,
            "Hybrid must report LexicalOnly in diagnostics, not Hybrid"
        );
    }

    #[tokio::test]
    async fn metadata_filter_narrows_results() {
        use crate::retrieval::MetadataFilter;

        let store = Arc::new(InMemoryDocumentStore::new());
        let chunker = ParagraphChunker {
            max_chunk_size: 500,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker);

        // Ingest two docs with different source types.
        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_plain"),
                source_id: SourceId::new("src_a"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Rust ownership model ensures memory safety.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_md"),
                source_id: SourceId::new("src_b"),
                source_type: SourceType::Markdown,
                project: ProjectKey::new("t", "w", "p"),
                content: "Rust ownership provides fearless concurrency.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let retrieval = InMemoryRetrieval::new(store);

        // Without filter: both docs match "Rust ownership".
        let all = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "Rust ownership".to_owned(),
                mode: RetrievalMode::LexicalOnly,
                reranker: RerankerStrategy::None,
                limit: 10,
                metadata_filters: vec![],
                scoring_policy: None,
            })
            .await
            .unwrap();
        assert_eq!(all.results.len(), 2);

        // With filter on source_type=PlainText: only plain text doc matches.
        let filtered = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "Rust ownership".to_owned(),
                mode: RetrievalMode::LexicalOnly,
                reranker: RerankerStrategy::None,
                limit: 10,
                metadata_filters: vec![MetadataFilter {
                    key: "source_type".to_owned(),
                    value: "PlainText".to_owned(),
                }],
                scoring_policy: None,
            })
            .await
            .unwrap();
        assert_eq!(filtered.results.len(), 1);
        assert_eq!(
            filtered.results[0].chunk.document_id,
            KnowledgeDocumentId::new("doc_plain")
        );
    }

    /// Verify retrieval diagnostics are fully populated per RFC 003.
    #[tokio::test]
    async fn diagnostics_fully_populated() {
        use crate::retrieval::CandidateStage;

        let store = Arc::new(InMemoryDocumentStore::new());
        let chunker = ParagraphChunker { max_chunk_size: 500 };
        let pipeline = IngestPipeline::new(store.clone(), chunker);

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_diag"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Diagnostics test content for retrieval.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let retrieval = InMemoryRetrieval::new(store);

        let response = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "diagnostics retrieval".to_owned(),
                mode: RetrievalMode::LexicalOnly,
                reranker: RerankerStrategy::None,
                limit: 5,
                metadata_filters: vec![],
                scoring_policy: None,
            })
            .await
            .unwrap();

        let diag = &response.diagnostics;

        // Mode and reranker reported.
        assert_eq!(diag.mode_used, RetrievalMode::LexicalOnly);
        assert_eq!(diag.reranker_used, RerankerStrategy::None);

        // Candidate-generation stages present.
        assert!(!diag.stages_used.is_empty());
        assert!(diag.stages_used.contains(&CandidateStage::Lexical));

        // Scoring dimensions that contributed are listed.
        assert!(
            diag.scoring_dimensions_used.contains(&"lexical_relevance".to_owned()),
            "lexical_relevance must always be listed"
        );
        assert!(
            diag.scoring_dimensions_used.contains(&"freshness".to_owned()),
            "freshness should be listed for recently-created chunks"
        );

        // Effective policy is described.
        let policy_str = diag.effective_policy.as_ref().expect("effective_policy should be Some");
        assert!(policy_str.contains("freshness_decay"));
        assert!(policy_str.contains("staleness_threshold"));
        assert!(policy_str.contains("recency="));

        // Counts are sane.
        assert!(diag.candidates_generated > 0);
        assert!(diag.results_returned > 0);
        assert!(diag.results_returned <= diag.candidates_generated);
        assert!(diag.latency_ms < 5000); // sanity: shouldn't take 5s

        // Per-result scoring breakdown is populated.
        for result in &response.results {
            assert!(result.breakdown.lexical_relevance > 0.0);
            assert!(result.breakdown.freshness > 0.0);
            assert!(result.score > 0.0);
        }
    }
}
