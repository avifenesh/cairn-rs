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
    registered_sources: Mutex<HashMap<ProjectKey, HashSet<SourceId>>>,
    schedules: Mutex<HashMap<String, RefreshSchedule>>,
    /// Tracks the last embedding model used across all chunks.
    ///
    /// Used as the sentinel for detecting when the model changes and
    /// all accepted memories need re-embedding (Go PR #1223).
    last_embedding_model_id: Mutex<Option<String>>,
}

impl InMemoryDocumentStore {
    pub fn new() -> Self {
        Self {
            docs: Mutex::new(HashMap::new()),
            chunks: Mutex::new(Vec::new()),
            registered_sources: Mutex::new(HashMap::new()),
            schedules: Mutex::new(HashMap::new()),
            last_embedding_model_id: Mutex::new(None),
        }
    }

    /// Return the embedding model last used to embed chunks in this store.
    ///
    /// `None` when no chunks have been embedded yet.
    pub fn last_embedding_model_id(&self) -> Option<String> {
        self.last_embedding_model_id.lock().unwrap().clone()
    }

    /// Set the active embedding model sentinel.
    ///
    /// Called by the ingest pipeline after embedding a batch of chunks.
    pub fn set_embedding_model_id(&self, model_id: impl Into<String>) {
        *self.last_embedding_model_id.lock().unwrap() = Some(model_id.into());
    }

    /// Mark all embedded chunks for re-embedding with `new_model_id`.
    ///
    /// Ported from Go PR #1223 (`ReembedAll`): when the embedding model
    /// changes, all accepted memories become stale. This method:
    /// 1. Clears `embedding` on every chunk that has one.
    /// 2. Sets `needs_reembed = true` on those chunks.
    /// 3. Records the target model in `embedding_model_id` so the re-embed
    ///    job knows which model to use.
    /// 4. Updates the store sentinel to `new_model_id`.
    ///
    /// Returns the number of chunks marked.
    pub fn re_embed_all(&self, new_model_id: &str) -> usize {
        let mut chunks = self.chunks.lock().unwrap();
        let mut count = 0;
        for chunk in chunks.iter_mut() {
            if chunk.embedding.is_some() {
                chunk.embedding = None;
                chunk.needs_reembed = true;
                chunk.embedding_model_id = Some(new_model_id.to_owned());
                count += 1;
            }
        }
        // Update the sentinel even if no chunks existed yet, so future ingests
        // know which model is current.
        *self.last_embedding_model_id.lock().unwrap() = Some(new_model_id.to_owned());
        count
    }

    /// Return the number of chunks currently flagged as needing re-embedding.
    pub fn pending_reembed_count(&self) -> usize {
        self.chunks
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.needs_reembed)
            .count()
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
                .or_insert_with(|| {
                    (
                        &chunk.source_id,
                        chunk.text.clone(),
                        chunk.entities.clone(),
                        chunk.created_at,
                    )
                });
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
                    source_type: *source_type,
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
        let registered_sources = self.registered_sources.lock().unwrap();
        // Count distinct document_ids per source_id.
        let mut source_docs: std::collections::HashMap<String, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        let mut source_last_ts: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();
        let mut source_ids: std::collections::HashMap<String, cairn_domain::SourceId> =
            std::collections::HashMap::new();
        for c in chunks.iter().filter(|c| &c.project == project) {
            let key = c.source_id.as_str().to_owned();
            source_docs
                .entry(key.clone())
                .or_default()
                .insert(c.document_id.as_str().to_owned());
            let ts = source_last_ts.entry(key.clone()).or_insert(0);
            if c.created_at > *ts {
                *ts = c.created_at;
            }
            source_ids.entry(key).or_insert_with(|| c.source_id.clone());
        }

        if let Some(registered) = registered_sources.get(project) {
            for source_id in registered {
                source_ids
                    .entry(source_id.as_str().to_owned())
                    .or_insert_with(|| source_id.clone());
            }
        }

        let mut summaries = source_ids
            .into_iter()
            .map(|(key, source_id)| SourceSummary {
                source_id,
                document_count: source_docs.get(&key).map_or(0, |docs| docs.len() as u64),
                avg_quality_score: 0.0,
                last_ingested_at_ms: source_last_ts.get(&key).copied(),
            })
            .collect::<Vec<_>>();
        summaries.sort_by(|a, b| a.source_id.as_str().cmp(b.source_id.as_str()));
        summaries
    }

    /// Register a source so it appears before any documents are ingested.
    pub fn register_source(
        &self,
        project: &cairn_domain::ProjectKey,
        source_id: &cairn_domain::SourceId,
    ) -> SourceSummary {
        self.registered_sources
            .lock()
            .unwrap()
            .entry(project.clone())
            .or_default()
            .insert(source_id.clone());
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

        let mut registered_sources = self.registered_sources.lock().unwrap();
        let mut removed_registered = false;
        for sources in registered_sources.values_mut() {
            removed_registered |= sources.remove(source_id);
        }
        registered_sources.retain(|_, sources| !sources.is_empty());

        chunks.len() < before || removed_registered
    }

    /// Check if a source is active (has any chunks).
    pub fn is_source_active(&self, source_id: &cairn_domain::SourceId) -> bool {
        if self
            .registered_sources
            .lock()
            .unwrap()
            .values()
            .any(|sources| sources.contains(source_id))
        {
            return true;
        }
        let chunks = self.chunks.lock().unwrap();
        chunks.iter().any(|c| &c.source_id == source_id)
    }

    /// Get a refresh schedule for a source.
    pub fn get_refresh_schedule(
        &self,
        source_id: &cairn_domain::SourceId,
    ) -> Option<RefreshSchedule> {
        let schedules = self.schedules.lock().unwrap();
        schedules
            .values()
            .find(|s| s.source_id == *source_id)
            .cloned()
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
        let schedule = RefreshSchedule {
            schedule_id: format!("sched_{}_{}", source_id.as_str(), now),
            source_id: source_id.clone(),
            interval_ms,
            last_refresh_ms: None,
            enabled: true,
            refresh_url,
        };
        self.schedules
            .lock()
            .unwrap()
            .insert(schedule.schedule_id.clone(), schedule.clone());
        schedule
    }

    /// List all due refresh schedules (interval elapsed since last refresh).
    pub fn list_due_schedules(&self, now_ms: u64) -> Vec<RefreshSchedule> {
        let schedules = self.schedules.lock().unwrap();
        schedules
            .values()
            .filter(|s| {
                if !s.enabled {
                    return false;
                }
                match s.last_refresh_ms {
                    None => true, // never refreshed — always due
                    Some(last) => now_ms.saturating_sub(last) >= s.interval_ms,
                }
            })
            .cloned()
            .collect()
    }

    /// Update the last refresh timestamp for a schedule.
    pub fn update_last_refresh_ms(&self, schedule_id: &str, now_ms: u64) {
        let mut schedules = self.schedules.lock().unwrap();
        if let Some(sched) = schedules.get_mut(schedule_id) {
            sched.last_refresh_ms = Some(now_ms);
        }
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
        document_id: &KnowledgeDocumentId,
        _limit: usize,
    ) -> Result<Vec<crate::ingest::DocumentVersion>, IngestError> {
        // Only return a version if the document exists and has been ingested.
        let exists = self.docs.lock().unwrap().contains_key(document_id.as_str());
        if !exists {
            return Ok(vec![]);
        }

        // Derive version metadata from the document's chunks.
        // Chunks may be absent when the IngestPipeline deduplicated all of them
        // (same content hash already in the project) — the document is still
        // registered in `docs` and has a valid identity, so return a synthetic
        // version record rather than an empty list.
        let chunks = self.chunks.lock().unwrap();
        let doc_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.document_id == *document_id)
            .collect();

        let content_hash = doc_chunks
            .first()
            .and_then(|c| c.content_hash.clone())
            .unwrap_or_default();
        let ingested_at_ms = doc_chunks.iter().map(|c| c.created_at).min().unwrap_or(0);

        Ok(vec![crate::ingest::DocumentVersion {
            document_id: document_id.clone(),
            version: 1,
            content_hash,
            ingested_at_ms,
        }])
    }
}

/// In-memory retrieval service using simple substring matching.
///
/// Not production-grade — this is for testing and local dev only.
/// Uses case-insensitive substring matching for lexical search.
pub struct InMemoryRetrieval {
    store: std::sync::Arc<InMemoryDocumentStore>,
    graph: Option<std::sync::Arc<cairn_graph::in_memory::InMemoryGraphStore>>,
    embedder: Option<std::sync::Arc<dyn crate::pipeline::EmbeddingProvider>>,
    /// Tracks per-chunk last-retrieval timestamps for recency_of_use scoring.
    last_retrieved: Mutex<HashMap<String, u64>>,
}

impl InMemoryRetrieval {
    pub fn new(store: std::sync::Arc<InMemoryDocumentStore>) -> Self {
        Self {
            store,
            graph: None,
            embedder: None,
            last_retrieved: Mutex::new(HashMap::new()),
        }
    }

    /// Create with a diagnostics adapter (stub — diagnostics adapter is ignored in in-memory backend).
    pub fn with_diagnostics(
        store: std::sync::Arc<InMemoryDocumentStore>,
        _diagnostics: std::sync::Arc<dyn crate::diagnostics::DiagnosticsService>,
    ) -> Self {
        Self {
            store,
            graph: None,
            embedder: None,
            last_retrieved: Mutex::new(HashMap::new()),
        }
    }

    /// Attach a graph store for graph-proximity scoring.
    pub fn with_graph(
        mut self,
        graph: std::sync::Arc<cairn_graph::in_memory::InMemoryGraphStore>,
    ) -> Self {
        self.graph = Some(graph);
        self
    }

    /// Attach an embedding provider for vector and hybrid retrieval modes.
    pub fn with_embedder(
        mut self,
        embedder: std::sync::Arc<dyn crate::pipeline::EmbeddingProvider>,
    ) -> Self {
        self.embedder = Some(embedder);
        self
    }
}

#[async_trait]
impl RetrievalService for InMemoryRetrieval {
    async fn query(&self, query: RetrievalQuery) -> Result<RetrievalResponse, RetrievalError> {
        // Embed the query text for vector/hybrid modes.
        let query_embedding = if matches!(
            query.mode,
            RetrievalMode::VectorOnly | RetrievalMode::Hybrid
        ) {
            match &self.embedder {
                Some(e) => {
                    let emb = e
                        .embed(&query.query_text)
                        .await
                        .map_err(|e| RetrievalError::EmbeddingFailed(e.to_string()))?;
                    if emb.is_empty() {
                        None
                    } else {
                        Some(emb)
                    }
                }
                None if query.mode == RetrievalMode::VectorOnly => {
                    return Err(RetrievalError::Internal(
                        "VectorOnly mode requires an embedding provider on InMemoryRetrieval. \
                         Use LexicalOnly or configure an embedder with with_embedder()."
                            .to_owned(),
                    ));
                }
                None => None, // Hybrid without embedder → lexical fallback
            }
        } else {
            None
        };

        let effective_mode = match query.mode {
            RetrievalMode::Hybrid if query_embedding.is_none() => RetrievalMode::LexicalOnly,
            other => other,
        };

        let use_lexical = matches!(
            effective_mode,
            RetrievalMode::LexicalOnly | RetrievalMode::Hybrid
        );
        let use_vector = matches!(
            effective_mode,
            RetrievalMode::VectorOnly | RetrievalMode::Hybrid
        );

        let start = std::time::Instant::now();
        let chunks = self.store.all_chunks();
        let query_lower = query.query_text.to_lowercase();

        let words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(ChunkRecord, f64, f64)> = chunks
            .into_iter()
            .filter(|c| c.project == query.project)
            .filter(|c| {
                query.metadata_filters.iter().all(|f| {
                    c.provenance_metadata.as_ref().is_some_and(|m| {
                        if let Some(v) = m.get(&f.key).and_then(|v| v.as_str()) {
                            return v == f.value;
                        }
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
                let lexical_score = if use_lexical {
                    let text_lower = c.text.to_lowercase();
                    let matches = words.iter().filter(|w| text_lower.contains(*w)).count();
                    matches as f64 / words.len().max(1) as f64
                } else {
                    0.0
                };

                let semantic_score = if use_vector {
                    match (&query_embedding, &c.embedding) {
                        (Some(qe), Some(ce)) => {
                            crate::reranking::cosine_similarity(qe, ce).max(0.0)
                        }
                        _ => 0.0,
                    }
                } else {
                    0.0
                };

                match effective_mode {
                    RetrievalMode::LexicalOnly if lexical_score == 0.0 => None,
                    RetrievalMode::VectorOnly if semantic_score == 0.0 => None,
                    RetrievalMode::Hybrid if lexical_score == 0.0 && semantic_score == 0.0 => None,
                    _ => Some((c, lexical_score, semantic_score)),
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            let key = |item: &(ChunkRecord, f64, f64)| -> f64 {
                match effective_mode {
                    RetrievalMode::VectorOnly => item.2,
                    RetrievalMode::LexicalOnly => item.1,
                    RetrievalMode::Hybrid => item.1 + item.2,
                }
            };
            key(b)
                .partial_cmp(&key(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

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

        // Snapshot last-retrieval timestamps for recency_of_use scoring.
        let last_retrieved_snap = self.last_retrieved.lock().unwrap().clone();

        let mut results: Vec<RetrievalResult> = scored
            .into_iter()
            .map(|(chunk, lexical_score, semantic_score)| {
                let fresh =
                    retrieval::freshness_score(chunk.created_at, now, policy.freshness_decay_days);
                let stale = retrieval::staleness_penalty(
                    chunk.updated_at,
                    chunk.created_at,
                    now,
                    policy.staleness_threshold_days,
                );

                let credibility = chunk
                    .credibility_score
                    .map(|s| s.clamp(0.0, 1.0))
                    .unwrap_or(0.0);

                // Recency of use: None if never retrieved, tiered score otherwise.
                let recency_of_use =
                    last_retrieved_snap
                        .get(chunk.chunk_id.as_str())
                        .map(|&last_ms| {
                            let age_ms = now.saturating_sub(last_ms);
                            const HOUR_MS: u64 = 3_600_000;
                            const DAY_MS: u64 = 86_400_000;
                            const WEEK_MS: u64 = 7 * DAY_MS;
                            if age_ms <= HOUR_MS {
                                1.0
                            } else if age_ms <= DAY_MS {
                                0.5
                            } else if age_ms <= WEEK_MS {
                                0.25
                            } else {
                                0.1
                            }
                        });

                let breakdown = ScoringBreakdown {
                    semantic_relevance: semantic_score,
                    lexical_relevance: lexical_score,
                    freshness: fresh,
                    staleness_penalty: stale,
                    source_credibility: credibility,
                    corroboration: 0.0,
                    graph_proximity: 0.0,
                    recency_of_use,
                };

                let final_score = retrieval::compute_final_score(&breakdown, &policy.weights);

                RetrievalResult {
                    chunk,
                    score: final_score,
                    breakdown,
                }
            })
            .collect();

        // Apply corroboration: a chunk scores higher when independent sources confirm
        // the same fact. For each result, count how many other results from DIFFERENT
        // sources corroborate it. Two chunks corroborate if:
        // - Both have embeddings AND cosine similarity > 0.8; OR
        // - Either lacks an embedding AND both share ≥50% of query words.
        {
            let total_others = results.len().saturating_sub(1).max(1) as f64;

            // Pre-compute per-chunk query-word coverage for lexical fallback.
            let query_words: Vec<String> = query_lower
                .split_whitespace()
                .map(|w| w.to_owned())
                .collect();
            let word_count = query_words.len().max(1);

            struct ChunkInfo {
                source_id: String,
                matched_words: HashSet<String>,
                has_embedding: bool,
            }
            let infos: Vec<ChunkInfo> = results
                .iter()
                .map(|r| {
                    let text_lower = r.chunk.text.to_lowercase();
                    let matched: HashSet<String> = query_words
                        .iter()
                        .filter(|w| text_lower.contains(w.as_str()))
                        .cloned()
                        .collect();
                    ChunkInfo {
                        source_id: r.chunk.source_id.as_str().to_owned(),
                        matched_words: matched,
                        has_embedding: r.chunk.embedding.is_some(),
                    }
                })
                .collect();

            for i in 0..results.len() {
                let mut corroborating = 0usize;
                for j in 0..results.len() {
                    if i == j {
                        continue;
                    }
                    // Different sources only.
                    if infos[i].source_id == infos[j].source_id {
                        continue;
                    }
                    // Embedding path: both have embeddings → cosine > 0.8.
                    if infos[i].has_embedding && infos[j].has_embedding {
                        if let (Some(ei), Some(ej)) =
                            (&results[i].chunk.embedding, &results[j].chunk.embedding)
                        {
                            if crate::reranking::cosine_similarity(ei, ej) > 0.8 {
                                corroborating += 1;
                            }
                        }
                    } else {
                        // Lexical fallback: ≥50% shared query words.
                        let shared = infos[i]
                            .matched_words
                            .intersection(&infos[j].matched_words)
                            .count();
                        if shared * 2 >= word_count {
                            corroborating += 1;
                        }
                    }
                }
                if corroborating > 0 {
                    results[i].breakdown.corroboration =
                        (corroborating as f64 / total_others).min(1.0);
                    results[i].score =
                        retrieval::compute_final_score(&results[i].breakdown, &policy.weights);
                }
            }

            // Re-sort after corroboration update.
            results.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

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
                    result.score =
                        retrieval::compute_final_score(&result.breakdown, &policy.weights);
                }
            }

            // Re-sort after graph proximity update.
            results.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        // Track all dimensions that materially contributed across results.
        if results.iter().any(|r| r.breakdown.lexical_relevance != 0.0) {
            scoring_dims.push("lexical_relevance".to_owned());
        }
        if results
            .iter()
            .any(|r| r.breakdown.semantic_relevance != 0.0)
        {
            scoring_dims.push("semantic_relevance".to_owned());
        }
        if results.iter().any(|r| r.breakdown.freshness != 0.0) {
            scoring_dims.push("freshness".to_owned());
        }
        if results.iter().any(|r| r.breakdown.staleness_penalty != 0.0) {
            scoring_dims.push("staleness_penalty".to_owned());
        }
        if results
            .iter()
            .any(|r| r.breakdown.source_credibility != 0.0)
        {
            scoring_dims.push("source_credibility".to_owned());
        }
        if results.iter().any(|r| r.breakdown.corroboration != 0.0) {
            scoring_dims.push("corroboration".to_owned());
        }
        if results.iter().any(|r| r.breakdown.graph_proximity != 0.0) {
            scoring_dims.push("graph_proximity".to_owned());
        }
        if results
            .iter()
            .any(|r| r.breakdown.recency_of_use.unwrap_or(0.0) != 0.0)
        {
            scoring_dims.push("recency_of_use".to_owned());
        }

        // Re-sort by final weighted score.
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let candidates_generated = results.len();

        // Apply reranking if requested.
        let base_stages = match effective_mode {
            RetrievalMode::VectorOnly => vec![CandidateStage::Vector],
            RetrievalMode::Hybrid => vec![
                CandidateStage::Lexical,
                CandidateStage::Vector,
                CandidateStage::Merged,
            ],
            _ => vec![CandidateStage::Lexical],
        };

        let (results, stages) = match query.reranker {
            RerankerStrategy::Mmr => {
                let reranked = mmr_rerank(&results, query.limit, 0.5);
                let mut stages = base_stages;
                stages.push(CandidateStage::Reranked);
                (reranked, stages)
            }
            _ => {
                let mut r = results;
                r.truncate(query.limit);
                (r, base_stages)
            }
        };

        // Record retrieval timestamps for returned chunks so subsequent
        // queries see positive recency_of_use.
        {
            let record_now = retrieval::now_ms();
            let mut lr = self.last_retrieved.lock().unwrap();
            for r in &results {
                lr.insert(r.chunk.chunk_id.as_str().to_owned(), record_now);
            }
        }

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
    pub fn register_source(
        &self,
        project: &cairn_domain::ProjectKey,
        source_id: &cairn_domain::SourceId,
    ) -> SourceSummary {
        self.store.register_source(project, source_id)
    }

    /// List all known sources for a project.
    pub fn list_sources(&self, project: &cairn_domain::ProjectKey) -> Vec<SourceSummary> {
        // Delegate to the underlying store to get correct document counts.
        self.store.list_sources(project)
    }

    /// Check if a source is active (has any chunks).
    pub fn is_source_active(&self, source_id: &cairn_domain::SourceId) -> bool {
        self.store.is_source_active(source_id)
    }

    /// Deactivate a source (removes all its chunks).
    pub fn deactivate_source(&self, source_id: &cairn_domain::SourceId) -> bool {
        self.store.deactivate_source(source_id)
    }

    /// Get all chunks (alias for all_chunks used by diagnostics).
    pub fn all_current_chunks(&self) -> Vec<ChunkRecord> {
        self.store.all_chunks()
    }

    /// Create a refresh schedule for a source.
    pub fn create_refresh_schedule(
        &self,
        source_id: &cairn_domain::SourceId,
        project: &cairn_domain::ProjectKey,
        interval_ms: u64,
        refresh_url: Option<String>,
    ) -> RefreshSchedule {
        self.store
            .create_refresh_schedule(source_id, project, interval_ms, refresh_url)
    }

    /// Get a refresh schedule for a source.
    pub fn get_refresh_schedule(
        &self,
        source_id: &cairn_domain::SourceId,
    ) -> Option<RefreshSchedule> {
        self.store.get_refresh_schedule(source_id)
    }

    /// List all due refresh schedules.
    pub fn list_due_schedules(&self, now_ms: u64) -> Vec<RefreshSchedule> {
        self.store.list_due_schedules(now_ms)
    }

    /// Update the last refresh timestamp for a schedule.
    pub fn update_last_refresh_ms(&self, schedule_id: &str, now_ms: u64) {
        self.store.update_last_refresh_ms(schedule_id, now_ms);
    }
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
    use crate::retrieval::{CandidateStage, RerankerStrategy, RetrievalMode};
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
        let chunker = ParagraphChunker {
            max_chunk_size: 500,
        };
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
            diag.scoring_dimensions_used
                .contains(&"lexical_relevance".to_owned()),
            "lexical_relevance must always be listed"
        );
        assert!(
            diag.scoring_dimensions_used
                .contains(&"freshness".to_owned()),
            "freshness should be listed for recently-created chunks"
        );

        // Effective policy is described.
        let policy_str = diag
            .effective_policy
            .as_ref()
            .expect("effective_policy should be Some");
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

    // --- Vector and Hybrid retrieval tests (RFC 003) ---

    /// Deterministic mock embedding provider for tests.
    struct MockEmbedder;

    #[async_trait::async_trait]
    impl crate::pipeline::EmbeddingProvider for MockEmbedder {
        async fn embed(&self, text: &str) -> Result<Vec<f32>, crate::ingest::IngestError> {
            let bytes = text.as_bytes();
            let dim = 16;
            let mut embedding = vec![0.0f32; dim];
            for (i, b) in bytes.iter().enumerate() {
                embedding[i % dim] += *b as f32;
            }
            let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut embedding {
                    *v /= norm;
                }
            }
            Ok(embedding)
        }
    }

    /// VectorOnly mode works when an embedding provider is configured.
    #[tokio::test]
    async fn vector_only_with_embedder_returns_results() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let embedder: Arc<dyn crate::pipeline::EmbeddingProvider> = Arc::new(MockEmbedder);
        let chunker = ParagraphChunker {
            max_chunk_size: 500,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker).with_embedder(embedder.clone());

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_vec1"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Rust is a systems programming language focused on safety.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_vec2"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Python is a dynamic scripting language for data science.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        // Verify chunks have embeddings from the pipeline.
        let chunks = store.all_chunks();
        assert!(
            chunks.iter().all(|c| c.embedding.is_some()),
            "pipeline should embed chunks"
        );

        let retrieval = InMemoryRetrieval::new(store).with_embedder(embedder);

        let response = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "Rust systems programming safety".to_owned(),
                mode: RetrievalMode::VectorOnly,
                reranker: RerankerStrategy::None,
                limit: 5,
                metadata_filters: vec![],
                scoring_policy: None,
            })
            .await
            .unwrap();

        assert!(!response.results.is_empty());
        assert_eq!(response.diagnostics.mode_used, RetrievalMode::VectorOnly);
        assert!(response
            .diagnostics
            .stages_used
            .contains(&CandidateStage::Vector));

        // VectorOnly: semantic populated, lexical zero.
        for r in &response.results {
            assert!(r.breakdown.semantic_relevance > 0.0);
            assert_eq!(r.breakdown.lexical_relevance, 0.0);
        }
    }

    /// Hybrid mode with embedder combines lexical and vector scores.
    #[tokio::test]
    async fn hybrid_mode_with_embedder_combines_scores() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let embedder: Arc<dyn crate::pipeline::EmbeddingProvider> = Arc::new(MockEmbedder);
        let chunker = ParagraphChunker {
            max_chunk_size: 500,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker).with_embedder(embedder.clone());

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_hyb1"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Rust borrow checker ensures memory safety without garbage collection."
                    .to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let retrieval = InMemoryRetrieval::new(store).with_embedder(embedder);

        let response = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "borrow checker memory".to_owned(),
                mode: RetrievalMode::Hybrid,
                reranker: RerankerStrategy::None,
                limit: 5,
                metadata_filters: vec![],
                scoring_policy: None,
            })
            .await
            .unwrap();

        assert_eq!(response.diagnostics.mode_used, RetrievalMode::Hybrid);
        assert!(response
            .diagnostics
            .stages_used
            .contains(&CandidateStage::Lexical));
        assert!(response
            .diagnostics
            .stages_used
            .contains(&CandidateStage::Vector));
        assert!(response
            .diagnostics
            .stages_used
            .contains(&CandidateStage::Merged));

        // Both dimensions should be populated.
        for r in &response.results {
            assert!(
                r.breakdown.lexical_relevance > 0.0,
                "hybrid should have lexical score"
            );
            assert!(
                r.breakdown.semantic_relevance > 0.0,
                "hybrid should have semantic score"
            );
        }
    }

    /// VectorOnly returns more relevant results first (cosine ordering).
    #[tokio::test]
    async fn vector_only_ranks_by_similarity() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let embedder: Arc<dyn crate::pipeline::EmbeddingProvider> = Arc::new(MockEmbedder);
        let chunker = ParagraphChunker {
            max_chunk_size: 500,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker).with_embedder(embedder.clone());

        // Ingest two docs: one closely matching the query, one distant.
        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_close"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Rust systems programming language safety performance.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_distant"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Cooking recipes for Italian pasta and Mediterranean salads.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let retrieval = InMemoryRetrieval::new(store).with_embedder(embedder);

        let response = retrieval
            .query(RetrievalQuery {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "Rust systems programming language safety performance".to_owned(),
                mode: RetrievalMode::VectorOnly,
                reranker: RerankerStrategy::None,
                limit: 5,
                metadata_filters: vec![],
                scoring_policy: None,
            })
            .await
            .unwrap();

        assert!(response.results.len() >= 2);
        // The closely matching doc should score higher.
        assert_eq!(
            response.results[0].chunk.document_id,
            KnowledgeDocumentId::new("doc_close"),
            "closer semantic match should rank first"
        );
    }

    // ── Go PR #1223: ReembedAll — model sentinel + stale-embedding detection ──

    /// Pipeline stores the embedding model ID on each chunk at ingest time.
    #[tokio::test]
    async fn pipeline_stores_embedding_model_id_on_chunks() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let embedder: Arc<dyn crate::pipeline::EmbeddingProvider> = Arc::new(MockEmbedder);
        let chunker = ParagraphChunker {
            max_chunk_size: 500,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker)
            .with_embedder(embedder)
            .with_embedding_model_id("nomic-embed-v1.5");

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_emb_id"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Rust is a systems programming language.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let chunks = store.all_chunks();
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert_eq!(
                chunk.embedding_model_id.as_deref(),
                Some("nomic-embed-v1.5"),
                "model ID must be stored alongside every embedding"
            );
            assert!(chunk.embedding.is_some(), "embedding must be present");
            assert!(
                !chunk.needs_reembed,
                "fresh chunks must not be flagged for re-embedding"
            );
        }
    }

    /// When no model ID is configured, embedding_model_id stays None.
    #[tokio::test]
    async fn pipeline_without_model_id_leaves_field_none() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let embedder: Arc<dyn crate::pipeline::EmbeddingProvider> = Arc::new(MockEmbedder);
        let chunker = ParagraphChunker {
            max_chunk_size: 500,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker).with_embedder(embedder);
        // no with_embedding_model_id call

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_no_mid"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Python is a high-level language.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let chunks = store.all_chunks();
        for chunk in &chunks {
            assert!(chunk.embedding.is_some());
            assert!(
                chunk.embedding_model_id.is_none(),
                "no model ID should be stored when pipeline has none configured"
            );
        }
    }

    /// re_embed_all clears embeddings and flags chunks; sentinel is updated.
    #[tokio::test]
    async fn re_embed_all_marks_stale_chunks_and_updates_sentinel() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let embedder: Arc<dyn crate::pipeline::EmbeddingProvider> = Arc::new(MockEmbedder);
        let chunker = ParagraphChunker {
            max_chunk_size: 200,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker)
            .with_embedder(embedder)
            .with_embedding_model_id("nomic-embed-v1");

        // Ingest two documents so we have multiple embedded chunks.
        for (i, content) in [
            "Rust ownership model provides memory safety without GC.",
            "Python uses reference counting and a garbage collector.",
        ]
        .iter()
        .enumerate()
        {
            pipeline
                .submit(IngestRequest {
                    document_id: KnowledgeDocumentId::new(format!("doc_reemb_{i}")),
                    source_id: SourceId::new("src"),
                    source_type: SourceType::PlainText,
                    project: ProjectKey::new("t", "w", "p"),
                    content: content.to_string(),
                    import_id: None,
                    corpus_id: None,
                    bundle_source_id: None,
                    tags: vec![],
                })
                .await
                .unwrap();
        }

        let before_chunks = store.all_chunks();
        let embedded_count = before_chunks
            .iter()
            .filter(|c| c.embedding.is_some())
            .count();
        assert!(
            embedded_count > 0,
            "chunks must have embeddings before re_embed_all"
        );

        // Simulate model upgrade.
        let marked = store.re_embed_all("nomic-embed-v2");

        assert_eq!(
            marked, embedded_count,
            "re_embed_all must mark exactly the embedded chunks"
        );
        assert_eq!(
            store.last_embedding_model_id().as_deref(),
            Some("nomic-embed-v2"),
            "sentinel must be updated to the new model"
        );
        assert_eq!(
            store.pending_reembed_count(),
            embedded_count,
            "all previously embedded chunks must be pending re-embedding"
        );

        let after_chunks = store.all_chunks();
        for chunk in &after_chunks {
            if chunk.needs_reembed {
                assert!(
                    chunk.embedding.is_none(),
                    "embedding must be cleared on re-embed flagged chunks"
                );
                assert_eq!(
                    chunk.embedding_model_id.as_deref(),
                    Some("nomic-embed-v2"),
                    "target model must be recorded on flagged chunks"
                );
            }
        }
    }

    /// Chunks without embeddings are not affected by re_embed_all.
    #[tokio::test]
    async fn re_embed_all_skips_unembedded_chunks() {
        let store = Arc::new(InMemoryDocumentStore::new());
        // Use a no-op embedder (default) — chunks get no embeddings.
        let chunker = ParagraphChunker {
            max_chunk_size: 200,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker);

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_nonemb"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "No embedder configured — chunks stay unembedded.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let marked = store.re_embed_all("new-model");
        assert_eq!(
            marked, 0,
            "no chunks should be marked when none have embeddings"
        );
        assert_eq!(store.pending_reembed_count(), 0);
        // Sentinel still updated so future ingests use the new model.
        assert_eq!(
            store.last_embedding_model_id().as_deref(),
            Some("new-model")
        );
    }
}
