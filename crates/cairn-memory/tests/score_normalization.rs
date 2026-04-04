//! RFC 003 retrieval quality score normalization tests.
//!
//! After the full scoring pipeline, scores are normalized so the top
//! result = 1.0 and all others are relative fractions in [0, 1].

use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{
    RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService, ScoringWeights,
};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// After a query, all returned scores must be in (0, 1] and
/// the highest-scoring result must have score exactly 1.0.
#[tokio::test]
async fn score_normalization_top_result_is_one() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    // Ingest docs with different lexical match counts to create score variance.
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_high"),
            source_id: SourceId::new("src_a"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust ownership memory safety borrow checker fearless concurrency".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_mid"),
            source_id: SourceId::new("src_b"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust ownership memory safety is important for systems beta".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_low"),
            source_id: SourceId::new("src_c"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust ownership is good gamma edition document content".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust ownership memory safety".to_owned(),
            query_embedding: None,
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(response.results.len() >= 2, "need at least 2 results to test normalization");

    // ALL scores must be in (0, 1].
    for result in &response.results {
        assert!(
            result.score > 0.0 && result.score <= 1.0,
            "score must be in (0, 1], got {} for chunk {}",
            result.score,
            result.chunk.chunk_id
        );
    }

    // The TOP result must have score = 1.0 exactly (after normalization).
    let top_score = response.results[0].score;
    assert!(
        (top_score - 1.0).abs() < 1e-9,
        "top result must have score=1.0 after normalization, got {top_score}"
    );
}

/// Relative ordering is preserved after normalization.
/// If result A scored higher than B before, A still scores higher after.
#[tokio::test]
async fn score_normalization_preserves_ordering() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("ord_a"),
            source_id: SourceId::new("src_a"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust safety ownership memory borrow checker ordering alpha".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("ord_b"),
            source_id: SourceId::new("src_b"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust safety ownership memory ordering beta edition".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("ord_c"),
            source_id: SourceId::new("src_c"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust safety ownership ordering gamma document".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust safety ownership memory".to_owned(),
            query_embedding: None,
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(response.results.len() >= 2);

    // Results returned in descending score order (sort was applied before normalization).
    for window in response.results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "results must be sorted descending: {} >= {}",
            window[0].score,
            window[1].score
        );
    }

    // Top score is 1.0.
    assert!((response.results[0].score - 1.0).abs() < 1e-9);

    // All other scores are < 1.0 (relative fractions), except ties.
    // This holds as long as the query produces different lexical scores for each doc.
}

/// validate_scoring_weights: default weights sum to 1.0 (no warning).
#[test]
fn score_normalization_default_weights_are_valid() {
    let weights = ScoringWeights::default();
    let valid = cairn_memory::retrieval::validate_scoring_weights(&weights);
    assert!(valid, "default ScoringWeights must sum to ~1.0");
}

/// normalize_weights: brings over-weighted policy back to sum=1.0.
#[test]
fn score_normalization_normalize_weights_corrects_sum() {
    let mut weights = ScoringWeights {
        semantic_weight: 1.0,
        lexical_weight: 1.0,
        freshness_weight: 0.5,
        staleness_weight: 0.5,
        credibility_weight: 0.5,
        corroboration_weight: 0.5,
        graph_proximity_weight: 0.5,
        recency_weight: 0.5,
    };
    // Sum = 5.0 (way over 1.0).
    assert!(cairn_memory::retrieval::weights_sum(&weights) > 1.01);

    cairn_memory::retrieval::normalize_weights(&mut weights);

    let new_sum = cairn_memory::retrieval::weights_sum(&weights);
    assert!(
        (new_sum - 1.0).abs() < 1e-9,
        "after normalize_weights, sum must be 1.0, got {new_sum}"
    );
}

/// Custom over-weighted ScoringPolicy gets normalized transparently.
#[tokio::test]
async fn score_normalization_overweighted_policy_is_normalized() {
    use cairn_memory::retrieval::ScoringPolicy;

    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("pol_doc"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "scoring policy normalization test content unique words here".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // Use heavily over-weighted policy (sum >> 1.0).
    let over_policy = ScoringPolicy {
        weights: ScoringWeights {
            semantic_weight: 2.0,
            lexical_weight: 2.0,
            freshness_weight: 1.0,
            staleness_weight: 0.5,
            credibility_weight: 1.0,
            corroboration_weight: 0.5,
            graph_proximity_weight: 0.5,
            recency_weight: 0.5,
        },
        ..ScoringPolicy::default()
    };

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "scoring policy normalization".to_owned(),
            query_embedding: None,
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: Some(over_policy),
        })
        .await
        .unwrap();

    assert!(!response.results.is_empty());

    // Even with over-weighted policy, normalization keeps scores in (0, 1].
    for result in &response.results {
        assert!(
            result.score > 0.0 && result.score <= 1.0,
            "score must be in (0, 1] even with over-weighted policy, got {}",
            result.score
        );
    }
    assert!((response.results[0].score - 1.0).abs() < 1e-9);
}

/// normalize_final_scores: empty input is a no-op.
#[test]
fn score_normalization_empty_results_no_panic() {
    let mut results: Vec<cairn_memory::retrieval::RetrievalResult> = vec![];
    cairn_memory::retrieval::normalize_final_scores(&mut results);
    assert!(results.is_empty());
}
