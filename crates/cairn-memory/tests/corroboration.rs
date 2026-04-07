//! RFC 003 retrieval corroboration scoring integration tests.
//!
//! Corroboration: a chunk scores higher when multiple independent sources
//! confirm the same fact. Implemented as a cross-result pass that counts
//! results from DIFFERENT sources that share ≥ 50% of query terms (lexical
//! fallback) or have cosine similarity > 0.8 (embedding path).

use std::sync::Arc;

use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{ChunkRecord, SourceType};
use cairn_memory::pipeline::DocumentStore;
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

fn make_chunk(id: &str, source: &str, text: &str, embedding: Option<Vec<f32>>) -> ChunkRecord {
    ChunkRecord {
        chunk_id: ChunkId::new(id),
        document_id: KnowledgeDocumentId::new(id),
        source_id: SourceId::new(source),
        source_type: SourceType::PlainText,
        project: project(),
        text: text.to_owned(),
        position: 0,
        created_at: cairn_memory::retrieval::now_ms(),
        updated_at: None,
        provenance_metadata: None,
        credibility_score: None,
        graph_linkage: None,
        embedding,
        content_hash: None,
        entities: vec![],
        embedding_model_id: None,
        needs_reembed: false,
    }
}

/// Three chunks from three different sources all describe "Rust ownership safety".
/// One outlier from a fourth source mentions only "Rust" (1/3 query words).
///
/// Expected:
/// - The 3 similar chunks each have corroboration > 0 (they share ≥ 50% of query words).
/// - The outlier has corroboration = 0 (shares only 1/3 = 33% < 50% of query words).
/// - The 3 corroborated chunks score strictly higher than the outlier.
#[tokio::test]
async fn corroboration_three_sources_confirm_same_fact() {
    let store = Arc::new(InMemoryDocumentStore::new());

    store
        .insert_chunks(&[
            make_chunk(
                "c_a",
                "src_alpha",
                "Rust ownership model enforces memory safety at compile time.",
                None,
            ),
            make_chunk(
                "c_b",
                "src_beta",
                "The Rust ownership system guarantees safety without garbage collection.",
                None,
            ),
            make_chunk(
                "c_c",
                "src_gamma",
                "Ownership and safety are the core pillars of Rust design.",
                None,
            ),
            // Outlier: mentions "Rust" but not "ownership" or "safety".
            make_chunk(
                "c_out",
                "src_delta",
                "Rust is a compiled systems programming language.",
                None,
            ),
        ])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);
    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust ownership safety".to_owned(),

            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 4, "all 4 chunks should match");

    let find = |src: &str| {
        response
            .results
            .iter()
            .find(|r| r.chunk.source_id == SourceId::new(src))
            .unwrap_or_else(|| panic!("missing result for source {src}"))
    };

    let ra = find("src_alpha");
    let rb = find("src_beta");
    let rc = find("src_gamma");
    let outlier = find("src_delta");

    // All three similar chunks must have corroboration > 0.
    assert!(
        ra.breakdown.corroboration > 0.0,
        "src_alpha should be corroborated, got {}",
        ra.breakdown.corroboration
    );
    assert!(
        rb.breakdown.corroboration > 0.0,
        "src_beta should be corroborated, got {}",
        rb.breakdown.corroboration
    );
    assert!(
        rc.breakdown.corroboration > 0.0,
        "src_gamma should be corroborated, got {}",
        rc.breakdown.corroboration
    );

    // Outlier shares only 1/3 query words — corroboration stays 0.
    assert_eq!(
        outlier.breakdown.corroboration, 0.0,
        "outlier should have no corroboration, got {}",
        outlier.breakdown.corroboration
    );

    // Corroborated chunks must score strictly higher than the lone outlier.
    assert!(
        ra.score > outlier.score,
        "src_alpha (corr={:.3}) should outscore outlier (corr={:.3}): {:.4} vs {:.4}",
        ra.breakdown.corroboration,
        outlier.breakdown.corroboration,
        ra.score,
        outlier.score
    );
}

/// Corroboration is symmetric: if A corroborates B then B corroborates A,
/// and both receive equal corroboration scores.
#[tokio::test]
async fn corroboration_is_symmetric_between_two_sources() {
    let store = Arc::new(InMemoryDocumentStore::new());

    store
        .insert_chunks(&[
            make_chunk(
                "c1",
                "src_x",
                "Rust ownership memory safety prevents data races.",
                None,
            ),
            make_chunk(
                "c2",
                "src_y",
                "Rust ownership and memory safety are core language features.",
                None,
            ),
        ])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);
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

    assert_eq!(response.results.len(), 2);

    let c1 = response
        .results
        .iter()
        .find(|r| r.chunk.chunk_id == ChunkId::new("c1"))
        .unwrap();
    let c2 = response
        .results
        .iter()
        .find(|r| r.chunk.chunk_id == ChunkId::new("c2"))
        .unwrap();

    assert!(c1.breakdown.corroboration > 0.0, "c1 should be corroborated");
    assert!(c2.breakdown.corroboration > 0.0, "c2 should be corroborated");
    assert!(
        (c1.breakdown.corroboration - c2.breakdown.corroboration).abs() < 1e-9,
        "corroboration must be symmetric: c1={}, c2={}",
        c1.breakdown.corroboration,
        c2.breakdown.corroboration
    );
}

/// Same-source chunks do NOT corroborate each other — corroboration requires
/// independent sources.
#[tokio::test]
async fn corroboration_same_source_does_not_corroborate() {
    let store = Arc::new(InMemoryDocumentStore::new());

    store
        .insert_chunks(&[
            make_chunk(
                "c1",
                "same_source",
                "Rust ownership memory safety prevents data races.",
                None,
            ),
            make_chunk(
                "c2",
                "same_source",
                "Rust ownership and memory safety are core features.",
                None,
            ),
        ])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);
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

    assert_eq!(response.results.len(), 2);
    for result in &response.results {
        assert_eq!(
            result.breakdown.corroboration, 0.0,
            "same-source chunks must not corroborate each other"
        );
    }
}

/// Embedding-based corroboration: two chunks from different sources with
/// cosine similarity > 0.8 corroborate each other regardless of text overlap.
/// A third chunk with an orthogonal embedding (cosine = 0.0) does not.
#[tokio::test]
async fn corroboration_embedding_cosine_similarity() {
    let store = Arc::new(InMemoryDocumentStore::new());

    // Identical unit vectors → cosine = 1.0 > 0.8 → ea and eb corroborate.
    let emb_a: Vec<f32> = vec![1.0, 0.0, 0.0];
    let emb_b: Vec<f32> = vec![1.0, 0.0, 0.0];
    // Orthogonal to ea/eb → cosine = 0.0 < 0.8 → ec does NOT corroborate via embedding.
    let emb_c: Vec<f32> = vec![0.0, 1.0, 0.0];

    store
        .insert_chunks(&[
            make_chunk("e_a", "src_emb_a", "topic description here alpha", Some(emb_a)),
            make_chunk("e_b", "src_emb_b", "topic description here beta", Some(emb_b)),
            make_chunk("e_c", "src_emb_c", "topic description here gamma", Some(emb_c)),
        ])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);
    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "topic description".to_owned(),

            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 3);

    let ea = response
        .results
        .iter()
        .find(|r| r.chunk.chunk_id == ChunkId::new("e_a"))
        .unwrap();
    let eb = response
        .results
        .iter()
        .find(|r| r.chunk.chunk_id == ChunkId::new("e_b"))
        .unwrap();
    let ec = response
        .results
        .iter()
        .find(|r| r.chunk.chunk_id == ChunkId::new("e_c"))
        .unwrap();

    // ea and eb have identical embeddings (cosine 1.0 > 0.8) → corroborate each other.
    assert!(
        ea.breakdown.corroboration > 0.0,
        "e_a should be corroborated by e_b via embedding similarity"
    );
    assert!(
        eb.breakdown.corroboration > 0.0,
        "e_b should be corroborated by e_a via embedding similarity"
    );
    // ec is orthogonal to ea and eb — cosine = 0.0 < 0.8 — no embedding corroboration.
    // (lexical fallback also won't fire because embeddings ARE present for all three.)
    assert_eq!(
        ec.breakdown.corroboration, 0.0,
        "e_c has orthogonal embedding — should not be corroborated"
    );
}
