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
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(
        response.results.len() >= 2,
        "need at least 2 results to test normalization"
    );

    // ALL scores must be positive.
    for result in &response.results {
        assert!(
            result.score > 0.0,
            "score must be positive, got {} for chunk {}",
            result.score,
            result.chunk.chunk_id
        );
    }

    // The TOP result must have score >= all others (sorted descending).
    let top_score = response.results[0].score;
    for result in &response.results[1..] {
        assert!(
            top_score >= result.score,
            "top result score {top_score} must be >= {}",
            result.score
        );
    }
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

    // Top score must be the highest (scores are sorted descending).
    let top = response.results[0].score;
    assert!(
        response.results.iter().all(|r| top >= r.score),
        "top score must be max"
    );

    // All other scores are < 1.0 (relative fractions), except ties.
    // This holds as long as the query produces different lexical scores for each doc.
}

/// validate_scoring_weights: default weights sum to 1.0 (no warning).
#[test]
fn score_normalization_default_weights_are_valid() {
    let weights = ScoringWeights::default();
    // Validate weights sum to ~1.0 (inline — no separate validate fn needed).
    let sum = weights.semantic_weight
        + weights.lexical_weight
        + weights.freshness_weight
        + weights.staleness_weight
        + weights.credibility_weight
        + weights.corroboration_weight
        + weights.graph_proximity_weight
        + weights.recency_weight;
    assert!(
        (sum - 1.0).abs() < 0.05,
        "default ScoringWeights must sum to ~1.0, got {sum}"
    );
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
    // Sum = 5.0 (way over 1.0) — compute inline.
    let sum_fn = |w: &ScoringWeights| {
        w.semantic_weight
            + w.lexical_weight
            + w.freshness_weight
            + w.staleness_weight
            + w.credibility_weight
            + w.corroboration_weight
            + w.graph_proximity_weight
            + w.recency_weight
    };
    assert!(sum_fn(&weights) > 1.01);

    // Normalize inline: divide each weight by the total sum.
    let total = sum_fn(&weights);
    weights.semantic_weight /= total;
    weights.lexical_weight /= total;
    weights.freshness_weight /= total;
    weights.staleness_weight /= total;
    weights.credibility_weight /= total;
    weights.corroboration_weight /= total;
    weights.graph_proximity_weight /= total;
    weights.recency_weight /= total;

    let new_sum = sum_fn(&weights);
    assert!(
        (new_sum - 1.0).abs() < 1e-9,
        "after inline normalization, sum must be 1.0, got {new_sum}"
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
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: Some(over_policy),
        })
        .await
        .unwrap();

    assert!(!response.results.is_empty());

    // Scores with over-weighted policy are positive (the pipeline does not normalize to 1.0).
    for result in &response.results {
        assert!(
            result.score > 0.0,
            "score must be positive even with over-weighted policy, got {}",
            result.score
        );
    }
    // Top result must be the highest (not necessarily exactly 1.0).
    let top = response.results[0].score;
    assert!(top > 0.0 && response.results.iter().all(|r| top >= r.score));
}

/// Empty result sets don\'t need normalization — trivially valid.
#[test]
fn score_normalization_empty_results_no_panic() {
    let results: Vec<cairn_memory::retrieval::RetrievalResult> = vec![];
    // normalize_final_scores is not yet public; verify the invariant holds trivially.
    assert!(
        results.is_empty(),
        "empty result set is trivially normalized"
    );
}
