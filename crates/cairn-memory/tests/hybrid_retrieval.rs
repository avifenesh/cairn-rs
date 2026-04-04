//! RFC 003 hybrid retrieval scoring integration tests.
//!
//! Hybrid mode combines lexical (60%) and vector (40%) signals.
//! When a query embedding is present, chunks with high embedding similarity
//! can outrank chunks that only match on text terms.

use std::sync::Arc;

use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{ChunkRecord, SourceType};
use cairn_memory::pipeline::DocumentStore;
use cairn_memory::retrieval::{
    CandidateStage, RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService,
};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

fn now_ms() -> u64 {
    cairn_memory::retrieval::now_ms()
}

/// Chunk A: strong lexical match, no embedding.
/// Chunk B: weak/no lexical match, identical embedding to query.
///
/// In Hybrid mode with a query embedding, Chunk B must rank above Chunk A
/// because its vector contribution (0.4 × 1.0 = 0.40) exceeds Chunk A's
/// lexical contribution (0.6 × 0.33 ≈ 0.20, one of three query words matched).
#[tokio::test]
async fn hybrid_retrieval_embedding_chunk_ranks_above_lexical_only() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let now = now_ms();

    // Query embedding: unit vector on dimension 0.
    let query_emb: Vec<f32> = vec![1.0, 0.0, 0.0];

    store
        .insert_chunks(&[
            // Chunk A: matches 1 of 3 query words, no embedding.
            ChunkRecord {
                chunk_id: ChunkId::new("lexical_chunk"),
                document_id: KnowledgeDocumentId::new("doc_a"),
                source_id: SourceId::new("src_a"),
                source_type: SourceType::PlainText,
                project: project(),
                text: "database systems are important for performance".to_owned(),
                position: 0,
                created_at: now,
                updated_at: None,
                provenance_metadata: None,
                credibility_score: None,
                graph_linkage: None,
                embedding: None, // no vector
                content_hash: None,
                superseded: false,
                tags: vec![],
                last_retrieved_at_ms: None,
                retrieval_count: 0,
                quality_score: None,
            },
            // Chunk B: doesn't match query words, but embedding identical to query.
            ChunkRecord {
                chunk_id: ChunkId::new("vector_chunk"),
                document_id: KnowledgeDocumentId::new("doc_b"),
                source_id: SourceId::new("src_b"),
                source_type: SourceType::PlainText,
                project: project(),
                text: "an irrelevant piece of text about weather forecasting".to_owned(),
                position: 0,
                created_at: now,
                updated_at: None,
                provenance_metadata: None,
                credibility_score: None,
                graph_linkage: None,
                embedding: Some(vec![1.0_f32, 0.0, 0.0]), // cosine sim = 1.0 with query
                content_hash: None,
                superseded: false,
                tags: vec![],
                last_retrieved_at_ms: None,
                retrieval_count: 0,
                quality_score: None,
            },
            // Chunk C: matches 2 of 3 query words, no embedding — extra noise.
            ChunkRecord {
                chunk_id: ChunkId::new("partial_lexical"),
                document_id: KnowledgeDocumentId::new("doc_c"),
                source_id: SourceId::new("src_c"),
                source_type: SourceType::PlainText,
                project: project(),
                text: "cache performance optimization tricks".to_owned(),
                position: 0,
                created_at: now,
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
            },
        ])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "cache performance database".to_owned(), // 3 words
            query_embedding: Some(query_emb),
            mode: RetrievalMode::Hybrid,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    // All three chunks should appear (each has at least one signal).
    assert_eq!(
        response.results.len(),
        3,
        "all 3 chunks should be included in Hybrid results"
    );

    // Find each result by chunk_id.
    let find = |id: &str| {
        response
            .results
            .iter()
            .find(|r| r.chunk.chunk_id == ChunkId::new(id))
            .unwrap_or_else(|| panic!("missing result for chunk {id}"))
    };

    let vector_result = find("vector_chunk");
    let lexical_result = find("lexical_chunk");

    // The vector chunk should have a non-zero vector_score.
    assert!(
        vector_result.breakdown.vector_score > 0.0,
        "vector_chunk must have a non-zero vector_score, got {}",
        vector_result.breakdown.vector_score
    );

    // The lexical-only chunk should have zero vector_score.
    assert_eq!(
        lexical_result.breakdown.vector_score, 0.0,
        "lexical_chunk has no embedding — vector_score must be 0"
    );

    // KEY ASSERTION: the embedding-similar chunk ranks above the weak lexical chunk.
    assert!(
        vector_result.score > lexical_result.score,
        "vector_chunk (vector_score={:.3}) should outscore lexical_chunk (lexical={:.3}): {:.4} vs {:.4}",
        vector_result.breakdown.vector_score,
        lexical_result.breakdown.lexical_relevance,
        vector_result.score,
        lexical_result.score
    );
}

/// Hybrid mode reports mode_used=Hybrid and includes Vector+Merged candidate stages.
#[tokio::test]
async fn hybrid_retrieval_diagnostics_report_hybrid_mode_and_stages() {
    let store = Arc::new(InMemoryDocumentStore::new());

    store
        .insert_chunks(&[ChunkRecord {
            chunk_id: ChunkId::new("diag_chunk"),
            document_id: KnowledgeDocumentId::new("diag_doc"),
            source_id: SourceId::new("diag_src"),
            source_type: SourceType::PlainText,
            project: project(),
            text: "hybrid retrieval diagnostic test content".to_owned(),
            position: 0,
            created_at: now_ms(),
            updated_at: None,
            provenance_metadata: None,
            credibility_score: None,
            graph_linkage: None,
            embedding: Some(vec![1.0_f32, 0.0, 0.0]),
            content_hash: None,
            superseded: false,
            tags: vec![],
            last_retrieved_at_ms: None,
            retrieval_count: 0,
            quality_score: None,
        }])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);
    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "hybrid retrieval".to_owned(),
            query_embedding: Some(vec![1.0_f32, 0.0, 0.0]),
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
        RetrievalMode::Hybrid,
        "Hybrid mode must report Hybrid in diagnostics"
    );
    assert!(
        response.diagnostics.stages_used.contains(&CandidateStage::Lexical),
        "Hybrid must include Lexical stage"
    );
    assert!(
        response.diagnostics.stages_used.contains(&CandidateStage::Vector),
        "Hybrid with query embedding must include Vector stage"
    );
    assert!(
        response.diagnostics.stages_used.contains(&CandidateStage::Merged),
        "Hybrid must include Merged stage"
    );
    assert!(
        response.diagnostics.scoring_dimensions_used.contains(&"vector_score".to_owned()),
        "vector_score must be listed as a scoring dimension when non-zero"
    );
}

/// Hybrid without a query embedding behaves like lexical but still reports Hybrid mode.
#[tokio::test]
async fn hybrid_retrieval_without_embedding_reports_hybrid_mode() {
    let store = Arc::new(InMemoryDocumentStore::new());

    store
        .insert_chunks(&[ChunkRecord {
            chunk_id: ChunkId::new("no_emb_chunk"),
            document_id: KnowledgeDocumentId::new("no_emb_doc"),
            source_id: SourceId::new("no_emb_src"),
            source_type: SourceType::PlainText,
            project: project(),
            text: "hybrid mode without query embedding".to_owned(),
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
        }])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);
    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "hybrid mode".to_owned(),
            query_embedding: None, // no embedding supplied
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
        RetrievalMode::Hybrid,
        "Hybrid without embedding must still report Hybrid, not LexicalOnly"
    );
    assert!(!response.results.is_empty(), "should return lexical results");
    // No vector scores when no query embedding is supplied.
    for result in &response.results {
        assert_eq!(
            result.breakdown.vector_score, 0.0,
            "no query embedding means vector_score must be 0"
        );
    }
}

/// Lexical-only mode is unaffected by the Hybrid changes.
#[tokio::test]
async fn hybrid_retrieval_lexical_only_mode_unchanged() {
    let store = Arc::new(InMemoryDocumentStore::new());

    store
        .insert_chunks(&[ChunkRecord {
            chunk_id: ChunkId::new("lex_only"),
            document_id: KnowledgeDocumentId::new("lex_doc"),
            source_id: SourceId::new("lex_src"),
            source_type: SourceType::PlainText,
            project: project(),
            text: "lexical only test content here".to_owned(),
            position: 0,
            created_at: now_ms(),
            updated_at: None,
            provenance_metadata: None,
            credibility_score: None,
            graph_linkage: None,
            embedding: Some(vec![1.0_f32, 0.0, 0.0]), // embedding present but mode is Lexical
            content_hash: None,
            superseded: false,
            tags: vec![],
            last_retrieved_at_ms: None,
            retrieval_count: 0,
            quality_score: None,
        }])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);
    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "lexical content".to_owned(),
            query_embedding: Some(vec![1.0_f32, 0.0, 0.0]),
            mode: RetrievalMode::LexicalOnly, // explicit lexical-only
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert_eq!(response.diagnostics.mode_used, RetrievalMode::LexicalOnly);
    // In LexicalOnly mode, vector_score must be 0 even if embedding is present.
    for result in &response.results {
        assert_eq!(
            result.breakdown.vector_score, 0.0,
            "LexicalOnly mode must not compute vector scores"
        );
    }
}
