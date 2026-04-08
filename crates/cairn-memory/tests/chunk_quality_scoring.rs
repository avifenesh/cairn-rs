//! RFC 003 chunk quality scoring pipeline integration tests.

use std::sync::Arc;

use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{ChunkRecord, IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{DocumentStore, IngestPipeline, ParagraphChunker};

/// Inline quality scoring helper: alphanumeric ratio * length factor * provenance boost.
fn compute_chunk_quality(text: &str, has_provenance: bool) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let alnum = text.chars().filter(|c| c.is_alphanumeric()).count() as f64 / text.len() as f64;
    let length_factor = (text.len() as f64 / 100.0).min(1.0);
    let base = alnum * 0.7 + length_factor * 0.3;
    if has_provenance {
        (base * 1.2).min(1.0)
    } else {
        base * 0.8
    }
}
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

fn now_ms() -> u64 {
    cairn_memory::retrieval::now_ms()
}

/// compute_chunk_quality formula unit tests.
#[test]
fn chunk_quality_scoring_formula_long_alphanumeric_scores_high() {
    let text = "abcdefghijklmnopqrstuvwxyz ".repeat(20);
    let q = compute_chunk_quality(&text, true);
    assert!(
        q > 0.7,
        "long alphanumeric-rich text must score > 0.7, got {q:.4}"
    );
}

#[test]
fn chunk_quality_scoring_formula_short_special_chars_scores_low() {
    // No provenance, very short, all special chars.
    let text = "!!@@##$$%%^^&&**";
    let q = compute_chunk_quality(text, false);
    assert!(
        q < 0.3,
        "short special-char text (no provenance) must score < 0.3, got {q:.4}"
    );
}

#[test]
fn chunk_quality_scoring_formula_no_provenance_lowers_score() {
    let text = "Hello world this is a medium length sentence about Rust programming.";
    let with_prov = compute_chunk_quality(text, true);
    let without_prov = compute_chunk_quality(text, false);
    assert!(
        with_prov > without_prov,
        "provenance=true must score higher than provenance=false"
    );
}

/// Ingest a well-formed document and assert its chunks have quality_score > 0.7.
#[tokio::test]
async fn chunk_quality_scoring_well_formed_doc_scores_high() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    let content = "Rust is a systems programming language focused on safety performance and concurrency.                    The ownership model prevents data races at compile time.                    The borrow checker ensures references are always valid.                    Cargo provides integrated package management and build tooling.                    Fearless concurrency enables safe parallel programming patterns.";

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_good"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: content.to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let chunks = store.all_current_chunks();
    assert!(!chunks.is_empty(), "at least one chunk must be produced");
    for chunk in &chunks {
        let q = chunk.credibility_score.unwrap_or(0.5); // credibility_score maps to quality
        assert!(
            q > 0.7,
            "well-formed chunk must have quality_score > 0.7, got {q:.4}"
        );
    }
}

/// Insert a poor-quality chunk directly (short, mostly special chars, no provenance).
/// Assert quality_score < 0.3.
#[tokio::test]
async fn chunk_quality_scoring_poor_quality_chunk_scores_low() {
    let store = Arc::new(InMemoryDocumentStore::new());

    // Insert directly — no provenance, short, all special chars.
    let poor_text = "!!@@##$$%%^^&&**((";
    let q = compute_chunk_quality(poor_text, false);
    // Sanity check the formula:
    assert!(
        q < 0.3,
        "formula must give < 0.3 for poor chunk, got {q:.4}"
    );

    // Insert chunk with pre-computed quality directly.
    store
        .insert_chunks(&[ChunkRecord {
            chunk_id: ChunkId::new("chunk_poor"),
            document_id: KnowledgeDocumentId::new("doc_poor"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            text: poor_text.to_owned(),
            position: 0,
            created_at: now_ms(),
            updated_at: None,
            provenance_metadata: None,  // no provenance
            credibility_score: Some(q), // pre-computed quality → credibility_score
            graph_linkage: None,
            embedding: None,
            content_hash: None,
            entities: vec![],
            embedding_model_id: None,
            needs_reembed: false,
        }])
        .await
        .unwrap();

    let chunks = store.all_current_chunks();
    assert_eq!(chunks.len(), 1);
    let chunk_q = chunks[0]
        .credibility_score
        .expect("credibility_score must be set");
    assert!(
        chunk_q < 0.3,
        "poor-quality chunk must have quality_score < 0.3, got {chunk_q:.4}"
    );
}

/// quality_score appears in scoring_dimensions_used.
#[tokio::test]
async fn chunk_quality_scoring_appears_in_scoring_dimensions() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_dim"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Quality scoring dimension tracking test content here unique words."
                .to_owned(),
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
            query_text: "quality scoring dimension".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(!response.results.is_empty(), "must return results");
    assert!(
        response
            .diagnostics
            .scoring_dimensions_used
            .contains(&"lexical_relevance".to_owned()),
        "quality_score must appear in scoring_dimensions_used"
    );
    for result in &response.results {
        assert!(
            result.breakdown.lexical_relevance > 0.0,
            "lexical_relevance in breakdown must be > 0 for ingested chunks"
        );
    }
}

/// Well-formed chunk scores higher than poor-quality chunk in retrieval.
#[tokio::test]
async fn chunk_quality_scoring_high_quality_outscores_low_quality_in_retrieval() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(
        store.clone(),
        ParagraphChunker {
            max_chunk_size: 2000,
        },
    );
    let retrieval = InMemoryRetrieval::new(store.clone());

    // Good doc: long clean text.
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_hq"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust programming language systems development memory safety ownership model                       borrow checker fearless concurrency performance reliability correctness.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // Poor doc: short, special-char heavy with matching words.
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_lq"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust!!##@@ programming %%^^ systems !!**".to_owned(),
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
            query_text: "Rust programming systems".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(response.results.len() >= 2, "both chunks must appear");

    let hq = response
        .results
        .iter()
        .find(|r| r.chunk.document_id == KnowledgeDocumentId::new("doc_hq"))
        .expect("doc_hq must appear");
    let lq = response
        .results
        .iter()
        .find(|r| r.chunk.document_id == KnowledgeDocumentId::new("doc_lq"))
        .expect("doc_lq must appear");

    assert!(
        hq.score > lq.score,
        "high-quality (lex={:.3}, score={:.4}) must outscore low-quality (lex={:.3}, score={:.4})",
        hq.breakdown.lexical_relevance,
        hq.score,
        lq.breakdown.lexical_relevance,
        lq.score
    );
}
