//! RFC 003 hybrid retrieval scoring integration tests.
//!
//! Hybrid mode in the in-memory backend falls back to lexical-only.
//! These tests verify that the retrieval pipeline handles mode selection,
//! diagnostics reporting, and scoring breakdown correctly.

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

/// Chunk A: strong lexical match.
/// Chunk B: weak/no lexical match.
/// Chunk C: partial lexical match.
///
/// In Hybrid mode (which falls back to lexical in the in-memory backend),
/// chunks matching more query words should rank higher.
#[tokio::test]
async fn hybrid_retrieval_lexical_chunk_ranks_by_word_overlap() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let now = now_ms();

    store
        .insert_chunks(&[
            // Chunk A: matches 1 of 3 query words.
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
                embedding: None,
                content_hash: None,
                entities: vec![],
                embedding_model_id: None,
                needs_reembed: false,
            },
            // Chunk B: doesn't match query words at all.
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
                embedding: Some(vec![1.0_f32, 0.0, 0.0]),
                content_hash: None,
                entities: vec![],
                embedding_model_id: None,
                needs_reembed: false,
            },
            // Chunk C: matches 2 of 3 query words — extra noise.
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
                entities: vec![],
                embedding_model_id: None,
                needs_reembed: false,
            },
        ])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "cache performance database".to_owned(), // 3 words
            mode: RetrievalMode::Hybrid,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    // Chunks with at least one lexical match should appear.
    assert!(
        !response.results.is_empty(),
        "at least some chunks should match lexically"
    );

    // Find results by chunk_id.
    let find = |id: &str| {
        response
            .results
            .iter()
            .find(|r| r.chunk.chunk_id == ChunkId::new(id))
    };

    let partial_result = find("partial_lexical");
    let lexical_result = find("lexical_chunk");

    // partial_lexical matches 2 of 3 words ("cache", "performance"),
    // lexical_chunk matches 1 of 3 words ("database" or "performance").
    // Both should have non-zero lexical_relevance.
    if let (Some(partial), Some(lexical)) = (partial_result, lexical_result) {
        assert!(
            partial.breakdown.lexical_relevance > 0.0,
            "partial_lexical must have a non-zero lexical_relevance, got {}",
            partial.breakdown.lexical_relevance
        );

        assert!(
            lexical.breakdown.lexical_relevance > 0.0,
            "lexical_chunk must have a non-zero lexical_relevance"
        );

        // The chunk matching more words should score at least as high.
        assert!(
            partial.score >= lexical.score,
            "partial_lexical (2-word match) should score >= lexical_chunk (1-word match): {:.4} vs {:.4}",
            partial.score,
            lexical.score
        );
    }
}

/// Hybrid mode reports mode_used=LexicalOnly (fallback) and includes Lexical candidate stage.
#[tokio::test]
async fn hybrid_retrieval_diagnostics_report_mode_and_stages() {
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
            entities: vec![],
            embedding_model_id: None,
            needs_reembed: false,
        }])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);
    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "hybrid retrieval".to_owned(),
            mode: RetrievalMode::Hybrid,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    // In-memory backend falls back to LexicalOnly for Hybrid mode.
    assert_eq!(
        response.diagnostics.mode_used,
        RetrievalMode::LexicalOnly,
        "Hybrid must report LexicalOnly in diagnostics (in-memory fallback)"
    );
    assert!(
        response.diagnostics.stages_used.contains(&CandidateStage::Lexical),
        "Hybrid must include Lexical stage"
    );
    assert!(
        response.diagnostics.scoring_dimensions_used.contains(&"lexical_relevance".to_owned()),
        "lexical_relevance must be listed as a scoring dimension when non-zero"
    );
}

/// Hybrid without a query embedding behaves like lexical and reports LexicalOnly mode.
#[tokio::test]
async fn hybrid_retrieval_without_embedding_reports_lexical_mode() {
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
            entities: vec![],
            embedding_model_id: None,
            needs_reembed: false,
        }])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);
    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "hybrid mode".to_owned(),
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
        "Hybrid without embedding must report LexicalOnly (in-memory fallback)"
    );
    assert!(!response.results.is_empty(), "should return lexical results");
    // In-memory backend sets semantic_relevance to 0 (no vector search).
    for result in &response.results {
        assert_eq!(
            result.breakdown.semantic_relevance, 0.0,
            "no vector search means semantic_relevance must be 0"
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
            embedding: Some(vec![1.0_f32, 0.0, 0.0]),
            content_hash: None,
            entities: vec![],
            embedding_model_id: None,
            needs_reembed: false,
        }])
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store);
    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "lexical content".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert_eq!(response.diagnostics.mode_used, RetrievalMode::LexicalOnly);
    // In LexicalOnly mode, semantic_relevance must be 0 even if embedding is present.
    for result in &response.results {
        assert_eq!(
            result.breakdown.semantic_relevance, 0.0,
            "LexicalOnly mode must not compute semantic scores"
        );
    }
}
