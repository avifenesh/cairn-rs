//! Concrete iterative deep search implementation.
//!
//! Decomposes the query into sub-queries, runs retrieval for each hop,
//! applies quality gates, and merges results with deduplication.

use async_trait::async_trait;
use std::collections::HashSet;
use std::time::Instant;

use crate::deep_search::{
    DeepSearchError, DeepSearchHop, DeepSearchRequest, DeepSearchResponse, DeepSearchService,
    HopOutcome,
};
use crate::retrieval::{RerankerStrategy, RetrievalQuery, RetrievalResult, RetrievalService};

/// Quality gate configuration for deep search.
pub struct QualityGateConfig {
    /// Minimum score threshold for a hop to be considered sufficient.
    pub min_score_threshold: f64,
    /// Minimum number of results needed for sufficiency.
    pub min_results: usize,
}

impl Default for QualityGateConfig {
    fn default() -> Self {
        Self {
            min_score_threshold: 0.3,
            min_results: 2,
        }
    }
}

/// Query decomposer that generates sub-queries for each hop.
pub trait QueryDecomposer: Send + Sync {
    fn decompose(&self, query: &str, hop: u32, prior_results: &[RetrievalResult]) -> String;
}

/// Simple keyword-based query decomposer.
///
/// Hop 0: original query.
/// Hop 1+: appends terms from top prior results to expand coverage.
pub struct KeywordDecomposer;

impl QueryDecomposer for KeywordDecomposer {
    fn decompose(&self, query: &str, hop: u32, prior_results: &[RetrievalResult]) -> String {
        if hop == 0 || prior_results.is_empty() {
            return query.to_owned();
        }

        let mut expansion_terms: Vec<&str> = Vec::new();
        for result in prior_results.iter().take(3) {
            for word in result.chunk.text.split_whitespace().take(5) {
                let w = word.trim_matches(|c: char| !c.is_alphanumeric());
                if w.len() > 3 && !query.to_lowercase().contains(&w.to_lowercase()) {
                    expansion_terms.push(w);
                }
            }
        }

        expansion_terms.truncate(5);

        if expansion_terms.is_empty() {
            query.to_owned()
        } else {
            format!("{} {}", query, expansion_terms.join(" "))
        }
    }
}

/// Iterative deep search that chains retrieval hops.
pub struct IterativeDeepSearch<R: RetrievalService, D: QueryDecomposer> {
    retrieval: R,
    decomposer: D,
    quality_gate: QualityGateConfig,
}

impl<R: RetrievalService> IterativeDeepSearch<R, KeywordDecomposer> {
    pub fn new(retrieval: R) -> Self {
        Self {
            retrieval,
            decomposer: KeywordDecomposer,
            quality_gate: QualityGateConfig::default(),
        }
    }
}

impl<R: RetrievalService, D: QueryDecomposer> IterativeDeepSearch<R, D> {
    pub fn with_quality_gate(mut self, config: QualityGateConfig) -> Self {
        self.quality_gate = config;
        self
    }

    fn check_quality(&self, results: &[RetrievalResult]) -> HopOutcome {
        if results.is_empty() {
            return HopOutcome::Exhausted;
        }

        let above_threshold = results
            .iter()
            .filter(|r| r.score >= self.quality_gate.min_score_threshold)
            .count();

        if above_threshold >= self.quality_gate.min_results {
            HopOutcome::Sufficient
        } else {
            HopOutcome::NeedsExpansion
        }
    }
}

#[async_trait]
impl<R: RetrievalService + 'static, D: QueryDecomposer + 'static> DeepSearchService
    for IterativeDeepSearch<R, D>
{
    async fn search(
        &self,
        request: DeepSearchRequest,
    ) -> Result<DeepSearchResponse, DeepSearchError> {
        let overall_start = Instant::now();
        let mut hops: Vec<DeepSearchHop> = Vec::new();
        let mut all_results: Vec<RetrievalResult> = Vec::new();
        let mut seen_chunk_ids: HashSet<String> = HashSet::new();

        for hop_number in 0..request.max_hops {
            let hop_start = Instant::now();

            let sub_query =
                self.decomposer
                    .decompose(&request.query_text, hop_number, &all_results);

            let response = self
                .retrieval
                .query(RetrievalQuery {
                    project: request.project.clone(),
                    query_text: sub_query.clone(),
                    mode: request.mode,
                    reranker: RerankerStrategy::None,
                    limit: request.per_hop_limit,
                    metadata_filters: vec![],
                })
                .await
                .map_err(|e| DeepSearchError::RetrievalFailed(e.to_string()))?;

            let new_results: Vec<RetrievalResult> = response
                .results
                .into_iter()
                .filter(|r| seen_chunk_ids.insert(r.chunk.chunk_id.clone()))
                .collect();

            let outcome = self.check_quality(&new_results);
            let hop_latency = hop_start.elapsed().as_millis() as u64;

            hops.push(DeepSearchHop {
                hop_number,
                sub_query,
                outcome,
                results: new_results.clone(),
                latency_ms: hop_latency,
            });

            all_results.extend(new_results);

            match outcome {
                HopOutcome::Sufficient | HopOutcome::Exhausted => break,
                HopOutcome::NeedsExpansion => continue,
            }
        }

        all_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(DeepSearchResponse {
            hops,
            merged_results: all_results,
            total_latency_ms: overall_start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
    use crate::ingest::{IngestRequest, IngestService, SourceType};
    use crate::pipeline::{IngestPipeline, ParagraphChunker};
    use crate::retrieval::RetrievalMode;
    use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
    use std::sync::Arc;

    async fn setup_retrieval() -> InMemoryRetrieval {
        let store = Arc::new(InMemoryDocumentStore::new());
        let chunker = ParagraphChunker {
            max_chunk_size: 100,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker);

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_1"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Rust ownership model prevents data races.\n\n\
                          The borrow checker validates references at compile time.\n\n\
                          Lifetimes ensure references remain valid."
                    .to_owned(),
            })
            .await
            .unwrap();

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_2"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Memory safety without garbage collection.\n\n\
                          Zero-cost abstractions in systems programming.\n\n\
                          Fearless concurrency through ownership."
                    .to_owned(),
            })
            .await
            .unwrap();

        InMemoryRetrieval::new(store)
    }

    #[tokio::test]
    async fn deep_search_finds_results_across_hops() {
        let retrieval = setup_retrieval().await;
        let search = IterativeDeepSearch::new(retrieval);

        let response = search
            .search(DeepSearchRequest {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "ownership".to_owned(),
                max_hops: 3,
                per_hop_limit: 5,
                mode: RetrievalMode::LexicalOnly,
            })
            .await
            .unwrap();

        assert!(!response.hops.is_empty());
        assert!(!response.merged_results.is_empty());
    }

    #[tokio::test]
    async fn deep_search_deduplicates_across_hops() {
        let retrieval = setup_retrieval().await;
        let search = IterativeDeepSearch::new(retrieval).with_quality_gate(QualityGateConfig {
            min_score_threshold: 0.99,
            min_results: 100,
        });

        let response = search
            .search(DeepSearchRequest {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "rust".to_owned(),
                max_hops: 3,
                per_hop_limit: 10,
                mode: RetrievalMode::LexicalOnly,
            })
            .await
            .unwrap();

        let mut ids: HashSet<String> = HashSet::new();
        for result in &response.merged_results {
            assert!(ids.insert(result.chunk.chunk_id.clone()));
        }
    }

    #[tokio::test]
    async fn deep_search_stops_early_when_sufficient() {
        let retrieval = setup_retrieval().await;
        let search = IterativeDeepSearch::new(retrieval).with_quality_gate(QualityGateConfig {
            min_score_threshold: 0.0,
            min_results: 1,
        });

        let response = search
            .search(DeepSearchRequest {
                project: ProjectKey::new("t", "w", "p"),
                query_text: "ownership".to_owned(),
                max_hops: 10,
                per_hop_limit: 5,
                mode: RetrievalMode::LexicalOnly,
            })
            .await
            .unwrap();

        assert_eq!(response.hops.len(), 1);
        assert_eq!(response.hops[0].outcome, HopOutcome::Sufficient);
    }

    #[tokio::test]
    async fn deep_search_empty_project_returns_exhausted() {
        let retrieval = setup_retrieval().await;
        let search = IterativeDeepSearch::new(retrieval);

        let response = search
            .search(DeepSearchRequest {
                project: ProjectKey::new("other", "w", "p"),
                query_text: "anything".to_owned(),
                max_hops: 3,
                per_hop_limit: 5,
                mode: RetrievalMode::LexicalOnly,
            })
            .await
            .unwrap();

        assert_eq!(response.hops.len(), 1);
        assert_eq!(response.hops[0].outcome, HopOutcome::Exhausted);
        assert!(response.merged_results.is_empty());
    }
}
