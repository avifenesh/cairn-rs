//! RFC 003 chunk similarity search integration tests.
//!
//! Tests verify that retrieval correctly ranks similar chunks above dissimilar
//! ones using the existing RetrievalService::query() lexical search.

use std::sync::Arc;

use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{ChunkRecord, SourceType};
use cairn_memory::pipeline::DocumentStore;
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

fn now_ms() -> u64 {
    cairn_memory::retrieval::now_ms()
}

fn make_chunk(id: &str, doc: &str, text: &str) -> ChunkRecord {
    ChunkRecord {
        chunk_id: ChunkId::new(id),
        document_id: KnowledgeDocumentId::new(doc),
        source_id: SourceId::new("src"),
        source_type: SourceType::PlainText,
        project: project(),
        text: text.to_owned(),
        position: 0,
        created_at: now_ms(),
        updated_at: None,
        provenance_metadata: None,
        credibility_score: None,
        graph_linkage: None,
        embedding: None,
        content_hash: None,
        entities: vec![],
        embedding_model_id: None,
        needs_reembed: false,
    }
}

/// Two similar chunks + one dissimilar. Searching for the content of the
/// first chunk must rank the second (similar) chunk above the third (dissimilar).
#[tokio::test]
async fn chunk_similarity_similar_chunk_ranks_above_dissimilar() {
    let store = Arc::new(InMemoryDocumentStore::new());

    store
        .insert_chunks(&[
            make_chunk(
                "c_similar_a",
                "doc1",
                "Rust ownership memory safety borrow checker fearless concurrency",
            ),
            make_chunk(
                "c_similar_b",
                "doc2",
                "Rust ownership memory safety ensures correct programs without errors",
            ),
            make_chunk(
                "c_dissimilar",
                "doc3",
                "Spaghetti carbonara pasta recipe eggs cheese pancetta black pepper",
            ),
        ])
        .await
        .unwrap();

    let retrieval = Arc::new(InMemoryRetrieval::new(store.clone()));

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust ownership memory safety borrow checker fearless concurrency"
                .to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    let results = &response.results;

    // All three chunks should appear (or at least the two similar ones).
    assert!(
        results.len() >= 2,
        "should return at least 2 results, got {}",
        results.len()
    );

    // The query chunk itself (c_similar_a) will match best since it's identical text.
    // The similar chunk (c_similar_b) must rank above the dissimilar one (c_dissimilar),
    // if the dissimilar one appears at all.
    let similar_b_pos = results
        .iter()
        .position(|r| r.chunk.chunk_id == ChunkId::new("c_similar_b"));
    let dissimilar_pos = results
        .iter()
        .position(|r| r.chunk.chunk_id == ChunkId::new("c_dissimilar"));

    if let (Some(sim_pos), Some(dis_pos)) = (similar_b_pos, dissimilar_pos) {
        assert!(
            sim_pos < dis_pos,
            "c_similar_b (pos={sim_pos}) must rank above c_dissimilar (pos={dis_pos})"
        );
    }

    // If both are present, scores must be descending.
    if let (Some(sim_pos), Some(dis_pos)) = (similar_b_pos, dissimilar_pos) {
        assert!(
            results[sim_pos].score >= results[dis_pos].score,
            "similar chunk (score={:.4}) must score >= dissimilar (score={:.4})",
            results[sim_pos].score,
            results[dis_pos].score
        );
    }

    // c_similar_b must be present.
    assert!(
        similar_b_pos.is_some(),
        "c_similar_b must appear in results"
    );
}

/// Retrieval respects the limit parameter.
#[tokio::test]
async fn chunk_similarity_respects_top_k() {
    let store = Arc::new(InMemoryDocumentStore::new());

    store
        .insert_chunks(&[
            make_chunk("q", "doc", "Rust systems programming language"),
            make_chunk("a", "doc", "Rust systems programming language safety"),
            make_chunk("b", "doc", "Rust systems programming language ownership"),
            make_chunk("c", "doc", "Rust systems programming language memory"),
        ])
        .await
        .unwrap();

    let retrieval = Arc::new(InMemoryRetrieval::new(store.clone()));

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust systems programming language".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 2,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert_eq!(
        response.results.len(),
        2,
        "limit=2 must return exactly 2 results"
    );
}

/// Query for a nonexistent term returns empty.
#[tokio::test]
async fn chunk_similarity_unknown_chunk_returns_empty() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let retrieval = Arc::new(InMemoryRetrieval::new(store));

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "nonexistent_chunk_term_xyz".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(
        response.results.is_empty(),
        "nonexistent term must return empty results"
    );
}

/// Text-based similarity: Rust chunks rank above unrelated content.
#[tokio::test]
async fn chunk_similarity_similar_to_text_ranks_correctly() {
    let store = Arc::new(InMemoryDocumentStore::new());

    store
        .insert_chunks(&[
            make_chunk(
                "rust_a",
                "d1",
                "Rust programming language systems development memory safety",
            ),
            make_chunk(
                "rust_b",
                "d2",
                "Rust language safety memory management borrow checker",
            ),
            make_chunk(
                "pizza",
                "d3",
                "Margherita pizza tomato sauce mozzarella basil olive oil dough",
            ),
        ])
        .await
        .unwrap();

    let retrieval = Arc::new(InMemoryRetrieval::new(store.clone()));

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust memory safety programming language".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    let results = &response.results;

    // Rust chunks must rank above the pizza chunk.
    let rust_scores: Vec<f64> = results
        .iter()
        .filter(|r| r.chunk.chunk_id.as_str().starts_with("rust"))
        .map(|r| r.score)
        .collect();
    let pizza_score = results
        .iter()
        .find(|r| r.chunk.chunk_id.as_str() == "pizza")
        .map(|r| r.score);

    // Pizza may not match at all (no overlapping words), which is fine.
    if let Some(ps) = pizza_score {
        for &rs in &rust_scores {
            assert!(
                rs > ps,
                "rust chunk (score={rs:.4}) must score above pizza (score={ps:.4})"
            );
        }
    }

    // At least the Rust chunks should match.
    assert!(
        rust_scores.len() >= 2,
        "both Rust chunks should appear in results"
    );
}

/// Retrieval via MemoryApiImpl search delegates correctly.
#[tokio::test]
async fn chunk_similarity_api_search_delegates() {
    use cairn_api::memory_api::{MemoryEndpoints, MemorySearchQuery};
    use cairn_memory::api_impl::MemoryApiImpl;

    let store = Arc::new(InMemoryDocumentStore::new());
    store
        .insert_chunks(&[
            make_chunk(
                "api_q",
                "doc_api",
                "API test chunk Rust safety ownership unique",
            ),
            make_chunk(
                "api_a",
                "doc_api2",
                "API test chunk Rust safety ownership similar",
            ),
            make_chunk(
                "api_b",
                "doc_api3",
                "Completely unrelated content about cooking",
            ),
        ])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store.clone());
    let api = MemoryApiImpl::new(retrieval, store);

    let results = api
        .search(
            &project(),
            &MemorySearchQuery {
                q: "Rust safety ownership".to_owned(),
                limit: Some(5),
            },
        )
        .await
        .unwrap();

    assert!(!results.is_empty(), "must return results");
    // The Rust-related chunks must appear.
    assert!(
        results[0].content.contains("Rust") || results[0].content.contains("safety"),
        "top result must be Rust-related, got: {}",
        results[0].content
    );
}

/// Retrieval via MemoryApiImpl: searching by text returns scored results.
#[tokio::test]
async fn chunk_similarity_api_search_returns_scored() {
    use cairn_api::memory_api::{MemoryEndpoints, MemorySearchQuery};
    use cairn_memory::api_impl::MemoryApiImpl;

    let store = Arc::new(InMemoryDocumentStore::new());
    store
        .insert_chunks(&[
            make_chunk(
                "txt1",
                "d1",
                "ownership borrow checker Rust memory safety system",
            ),
            make_chunk("txt2", "d2", "carbonara pasta recipe Italian kitchen"),
        ])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store.clone());
    let api = MemoryApiImpl::new(retrieval, store);

    let results = api
        .search(
            &project(),
            &MemorySearchQuery {
                q: "Rust ownership memory safety".to_owned(),
                limit: Some(5),
            },
        )
        .await
        .unwrap();

    assert!(!results.is_empty());
    // The Rust chunk must rank first.
    assert!(
        results[0].content.contains("Rust") || results[0].content.contains("ownership"),
        "Rust chunk must rank first, got: {}",
        results[0].content
    );
    assert!(
        results[0].confidence.unwrap_or(0.0) > 0.0,
        "confidence (mapped from score) must be positive"
    );
}
