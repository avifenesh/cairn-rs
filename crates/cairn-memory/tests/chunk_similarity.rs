//! RFC 003 chunk similarity search integration tests.
//!
//! similarity_search(): finds similar chunks by embedding or character n-gram fallback.
//! similar_to_text(): finds chunks similar to a given text string.

use std::sync::Arc;

use cairn_api::memory_api::ChunkSimilarityEndpoints;
use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::api_impl::ChunkSimilarityApiImpl;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{ChunkRecord, SourceType};
use cairn_memory::pipeline::DocumentStore;

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
        superseded: false,
        tags: vec![],
        last_retrieved_at_ms: None,
        retrieval_count: 0,
        quality_score: None,
    }
}

/// Two similar chunks + one dissimilar. Searching by the first chunk must
/// rank the second (similar) chunk above the third (dissimilar) chunk.
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

    let results = retrieval.similarity_search(
        &KnowledgeDocumentId::new("doc1"),
        "c_similar_a",
        5,
    );

    assert_eq!(results.len(), 2, "should return 2 results (excluding query chunk)");

    // The similar chunk must rank first.
    assert_eq!(
        results[0].chunk.chunk_id,
        ChunkId::new("c_similar_b"),
        "c_similar_b must rank #1 (most similar to c_similar_a)"
    );
    // The dissimilar chunk must rank last.
    assert_eq!(
        results[1].chunk.chunk_id,
        ChunkId::new("c_dissimilar"),
        "c_dissimilar must rank last"
    );
    // Scores must be descending.
    assert!(
        results[0].score >= results[1].score,
        "scores must be descending: {} >= {}",
        results[0].score,
        results[1].score
    );
    // The similar chunk should score significantly higher than the dissimilar one.
    assert!(
        results[0].score > results[1].score,
        "similar chunk (score={:.4}) must score strictly above dissimilar (score={:.4})",
        results[0].score,
        results[1].score
    );
}

/// similarity_search returns at most top_k results.
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
    let results = retrieval.similarity_search(&KnowledgeDocumentId::new("doc"), "q", 2);

    assert_eq!(results.len(), 2, "top_k=2 must return exactly 2 results");
}

/// similarity_search returns empty when chunk_id not found.
#[tokio::test]
async fn chunk_similarity_unknown_chunk_returns_empty() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let retrieval = Arc::new(InMemoryRetrieval::new(store));
    let results = retrieval.similarity_search(
        &KnowledgeDocumentId::new("doc"),
        "nonexistent_chunk",
        5,
    );
    assert!(results.is_empty(), "nonexistent chunk must return empty results");
}

/// similar_to_text finds chunks with similar content.
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
    let results = retrieval.similar_to_text(
        "Rust memory safety programming language",
        &project(),
        3,
    );

    assert_eq!(results.len(), 3);
    // Rust chunks must rank above the pizza chunk.
    let rust_scores: Vec<f64> = results
        .iter()
        .filter(|r| r.chunk.chunk_id.as_str().starts_with("rust"))
        .map(|r| r.score)
        .collect();
    let pizza_score = results
        .iter()
        .find(|r| r.chunk.chunk_id.as_str() == "pizza")
        .map(|r| r.score)
        .unwrap();

    for &rs in &rust_scores {
        assert!(
            rs > pizza_score,
            "rust chunk (score={rs:.4}) must score above pizza (score={pizza_score:.4})"
        );
    }
}

/// API impl: ChunkSimilarityApiImpl delegates to InMemoryRetrieval correctly.
#[tokio::test]
async fn chunk_similarity_api_impl_similar_by_chunk_id() {
    let store = Arc::new(InMemoryDocumentStore::new());
    store
        .insert_chunks(&[
            make_chunk("api_q", "doc_api", "API test chunk Rust safety ownership unique"),
            make_chunk("api_a", "doc_api2", "API test chunk Rust safety ownership similar"),
            make_chunk("api_b", "doc_api3", "Completely unrelated content about cooking"),
        ])
        .await
        .unwrap();

    let retrieval = Arc::new(InMemoryRetrieval::new(store));
    let api = ChunkSimilarityApiImpl::new(retrieval);

    let results = api.similar_by_chunk_id("doc_api", "api_q", &project(), 5).await.unwrap();

    assert!(!results.is_empty(), "must return results");
    assert_eq!(results[0].chunk_id, "api_a", "api_a must rank first (most similar)");
}

/// API impl: similar_to_text endpoint.
#[tokio::test]
async fn chunk_similarity_api_impl_similar_to_text() {
    let store = Arc::new(InMemoryDocumentStore::new());
    store
        .insert_chunks(&[
            make_chunk("txt1", "d1", "ownership borrow checker Rust memory safety system"),
            make_chunk("txt2", "d2", "carbonara pasta recipe Italian kitchen"),
        ])
        .await
        .unwrap();

    let retrieval = Arc::new(InMemoryRetrieval::new(store));
    let api = ChunkSimilarityApiImpl::new(retrieval);

    let results = api.similar_to_text("Rust ownership memory safety", &project(), 5).await.unwrap();

    assert!(results.len() >= 1);
    // The Rust chunk must rank first.
    assert_eq!(results[0].chunk_id, "txt1", "Rust chunk must rank first");
    assert!(
        results[0].similarity_score > 0.0,
        "similarity_score must be positive"
    );
}
