//! RFC 003 memory retrieval quality integration tests.
//!
//! Validates the retrieval pipeline against quality requirements:
//! - PlainText, Markdown, and Html source types all produce chunks.
//! - Source quality is tracked per-source via InMemoryDiagnostics.
//! - Retrieval results are ranked by lexical relevance score.
//! - Project-scoped isolation: queries must not leak across projects.
//! - Fresh documents score higher on the freshness dimension than stale ones.

use std::sync::Arc;

use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::diagnostics::DiagnosticsService;
use cairn_memory::diagnostics_impl::InMemoryDiagnostics;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{ChunkRecord, IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{compute_content_hash, DocumentStore, IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{
    freshness_score, RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService,
    ScoringPolicy, ScoringWeights,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_rq", "ws_rq", "proj_rq")
}

fn other_project() -> ProjectKey {
    ProjectKey::new("tenant_rq", "ws_rq", "proj_other")
}

fn source(n: &str) -> SourceId {
    SourceId::new(format!("src_{n}"))
}

fn doc(n: &str) -> KnowledgeDocumentId {
    KnowledgeDocumentId::new(format!("doc_{n}"))
}

fn ingest(project: ProjectKey, id: &str, source_id: &str, source_type: SourceType, content: &str) -> IngestRequest {
    IngestRequest {
        document_id: doc(id),
        source_id: source(source_id),
        source_type,
        project,
        content: content.to_owned(),
        tags: vec![],
        corpus_id: None,
        bundle_source_id: None,
        import_id: None,
    }
}

// ── (1) + (2): Ingest 3 source types, verify chunk extraction ─────────────────

/// (1) Ingest PlainText, Markdown, and Html documents.
/// (2) All three source types produce chunks in the store.
#[tokio::test]
async fn three_source_types_all_produce_chunks() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    // PlainText document.
    pipeline
        .submit(ingest(
            project(),
            "plain",
            "src_plain",
            SourceType::PlainText,
            "Rust is a systems programming language that provides memory safety without GC. \
             It uses ownership and borrowing to manage resources at compile time.",
        ))
        .await
        .unwrap();

    // Markdown document.
    pipeline
        .submit(ingest(
            project(),
            "markdown",
            "src_markdown",
            SourceType::Markdown,
            "# Async Rust\n\nAsync functions return a `Future`. \
             Use `tokio::spawn` to run tasks concurrently.\n\n\
             ## Error Handling\n\nUse the `?` operator to propagate errors in async functions.",
        ))
        .await
        .unwrap();

    // Html document.
    pipeline
        .submit(ingest(
            project(),
            "html",
            "src_html",
            SourceType::Html,
            "<h1>Cargo</h1><p>Cargo is the Rust package manager. \
             Use <code>cargo build</code> to compile and <code>cargo test</code> to run tests.</p>",
        ))
        .await
        .unwrap();

    let all_chunks = store.all_chunks();
    assert!(!all_chunks.is_empty(), "chunks must be produced for all three source types");

    // Each document type must have at least one chunk.
    let plain_chunks: Vec<_> = all_chunks.iter().filter(|c| c.document_id == doc("plain")).collect();
    let md_chunks: Vec<_>    = all_chunks.iter().filter(|c| c.document_id == doc("markdown")).collect();
    let html_chunks: Vec<_>  = all_chunks.iter().filter(|c| c.document_id == doc("html")).collect();

    assert!(!plain_chunks.is_empty(), "PlainText document must produce chunks");
    assert!(!md_chunks.is_empty(),    "Markdown document must produce chunks");
    assert!(!html_chunks.is_empty(),  "Html document must produce chunks");

    // Each chunk must carry its source type and project key.
    for chunk in &plain_chunks {
        assert_eq!(chunk.source_type, SourceType::PlainText);
        assert_eq!(chunk.project, project());
    }
    for chunk in &md_chunks {
        assert_eq!(chunk.source_type, SourceType::Markdown);
    }
    for chunk in &html_chunks {
        assert_eq!(chunk.source_type, SourceType::Html);
    }

    // All chunks carry a non-empty content hash.
    for chunk in &all_chunks {
        assert!(
            chunk.content_hash.as_ref().map_or(false, |h| !h.is_empty()),
            "every chunk must carry a non-empty content_hash"
        );
    }
}

// ── (3): Source quality via InMemoryDiagnostics ────────────────────────────────

/// (3) Source quality is tracked per source via InMemoryDiagnostics.
/// After recording ingests and retrieval hits, source_quality reflects them.
#[tokio::test]
async fn source_quality_tracks_each_source_independently() {
    let diag = InMemoryDiagnostics::new();

    // Record ingests from two sources.
    diag.record_ingest(&source("plain"),    &project(), 4);
    diag.record_ingest(&source("markdown"), &project(), 6);

    // Record retrieval hits only for the plain source.
    diag.record_retrieval_hit(&source("plain"), 0.95);
    diag.record_retrieval_hit(&source("plain"), 0.88);
    diag.record_retrieval_hit(&source("plain"), 0.72);

    // --- Plain source ---
    let plain_q = diag.source_quality(&source("plain")).await.unwrap().unwrap();
    assert_eq!(plain_q.total_chunks, 4);
    assert_eq!(plain_q.total_retrievals, 3, "plain source must record 3 retrieval hits");
    let expected_avg = (0.95 + 0.88 + 0.72) / 3.0;
    assert!(
        (plain_q.avg_relevance_score - expected_avg).abs() < 0.001,
        "plain source avg relevance must be {:.3}, got {:.3}",
        expected_avg, plain_q.avg_relevance_score
    );

    // --- Markdown source ---
    let md_q = diag.source_quality(&source("markdown")).await.unwrap().unwrap();
    assert_eq!(md_q.total_chunks, 6);
    assert_eq!(md_q.total_retrievals, 0, "markdown source has no retrieval hits yet");

    // --- HTML source (never recorded) ---
    let html_q = diag.source_quality(&source("html")).await.unwrap();
    assert!(html_q.is_none(), "unrecorded source must return None from source_quality");

    // Index status aggregates across all recorded sources for the project.
    let idx = diag.index_status(&project()).await.unwrap();
    assert_eq!(idx.total_documents, 2, "index must count 2 distinct document sources");
    assert_eq!(idx.total_chunks, 10, "index must sum chunks: 4 + 6 = 10");

    // list_source_quality returns both sources.
    let all_quality = diag.list_source_quality(&project(), 10).await.unwrap();
    assert_eq!(all_quality.len(), 2, "both sources must appear in list_source_quality");
}

// ── (4): Retrieval ranking by relevance ────────────────────────────────────────

/// (4) Results are ranked by lexical relevance — documents with more query
/// word matches score higher and appear earlier.
#[tokio::test]
async fn retrieval_ranking_by_relevance() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    // High-relevance doc: mentions all three query terms.
    pipeline
        .submit(ingest(
            project(),
            "high_rel",
            "src_high",
            SourceType::PlainText,
            "Rust async tokio runtime provides excellent performance for concurrent applications. \
             Tokio is the most popular async runtime in the Rust ecosystem.",
        ))
        .await
        .unwrap();

    // Low-relevance doc: mentions only one query term.
    pipeline
        .submit(ingest(
            project(),
            "low_rel",
            "src_low",
            SourceType::PlainText,
            "Python is a dynamic language popular for data science and machine learning workflows.",
        ))
        .await
        .unwrap();

    // Medium-relevance doc: mentions two query terms.
    pipeline
        .submit(ingest(
            project(),
            "med_rel",
            "src_med",
            SourceType::PlainText,
            "Async programming in Rust requires understanding futures and the async/await syntax. \
             The tokio crate is widely used.",
        ))
        .await
        .unwrap();

    // Query with terms that appear at different frequencies across docs.
    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "async tokio rust".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(!response.results.is_empty(), "query must return results");

    // Python-only doc must not rank above docs that mention all three terms.
    // Results are sorted by descending score — verify ordering by checking
    // the top result is not the Python document.
    let top_result_doc = &response.results[0].chunk.document_id;
    assert_ne!(
        *top_result_doc,
        doc("low_rel"),
        "the low-relevance Python doc must not be the top result"
    );

    // Scores must be in descending order.
    let scores: Vec<f64> = response.results.iter().map(|r| r.score).collect();
    for window in scores.windows(2) {
        assert!(
            window[0] >= window[1],
            "results must be sorted in descending score order: {} >= {}",
            window[0], window[1]
        );
    }

    // Diagnostics must report lexical_relevance as a used dimension.
    assert!(
        response.diagnostics.scoring_dimensions_used.contains(&"lexical_relevance".to_owned()),
        "lexical_relevance must be listed as a used scoring dimension"
    );
}

// ── (5): Project-scoped retrieval isolation ────────────────────────────────────

/// (5) Queries are scoped to the project — documents from another project
/// must not appear in results.
#[tokio::test]
async fn retrieval_is_project_scoped() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    let shared_term = "ownership borrowing lifetimes";

    // Ingest same topic into two different projects.
    pipeline
        .submit(ingest(
            project(),
            "proj_a_doc",
            "src_a",
            SourceType::PlainText,
            "Rust ownership borrowing and lifetimes are core language concepts.",
        ))
        .await
        .unwrap();

    pipeline
        .submit(ingest(
            other_project(),
            "proj_b_doc",
            "src_b",
            SourceType::PlainText,
            "Rust ownership borrowing and lifetimes require careful understanding.",
        ))
        .await
        .unwrap();

    // Query from project — must only return project results.
    let proj_a_results = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: shared_term.to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 20,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(!proj_a_results.results.is_empty(), "project must find its own document");
    assert!(
        proj_a_results.results.iter().all(|r| r.chunk.project == project()),
        "all results must be scoped to project"
    );
    assert!(
        !proj_a_results.results.iter().any(|r| r.chunk.document_id == doc("proj_b_doc")),
        "project query must not return documents from other_project"
    );

    // Query from other_project — must only see its own docs.
    let proj_b_results = retrieval
        .query(RetrievalQuery {
            project: other_project(),
            query_text: shared_term.to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 20,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(
        proj_b_results.results.iter().all(|r| r.chunk.project == other_project()),
        "other_project results must only include other_project documents"
    );
    assert!(
        !proj_b_results.results.iter().any(|r| r.chunk.document_id == doc("proj_a_doc")),
        "other_project query must not return documents from project"
    );
}

// ── (6): Stale documents get lower freshness scores ────────────────────────────

/// (6) A chunk with an old `created_at` timestamp scores lower on the
/// freshness dimension than a chunk ingested recently.
///
/// We bypass the pipeline and insert chunks directly with controlled
/// timestamps to produce a deterministic comparison.
#[tokio::test]
async fn stale_chunks_score_lower_on_freshness() {
    // Direct freshness_score comparison using the public scoring function.
    // This validates that the scoring function used by InMemoryRetrieval
    // behaves correctly for fresh vs. stale content.
    let now = cairn_memory::retrieval::now_ms();
    let decay_days = 30.0;

    // Fresh chunk: created 1 hour ago.
    let one_hour_ms: u64 = 3_600_000;
    let fresh_created_at = now.saturating_sub(one_hour_ms);
    let fresh_score = freshness_score(fresh_created_at, now, decay_days);

    // Stale chunk: created 120 days ago (4× the decay half-life).
    let one_twenty_days_ms: u64 = 120 * 86_400_000;
    let stale_created_at = now.saturating_sub(one_twenty_days_ms);
    let stale_score = freshness_score(stale_created_at, now, decay_days);

    assert!(
        fresh_score > 0.99,
        "chunk from 1 hour ago must be nearly maximally fresh (got {fresh_score:.4})"
    );
    assert!(
        stale_score < 0.02,
        "chunk from 120 days ago must be nearly minimally fresh with 30-day decay (got {stale_score:.4})"
    );
    assert!(
        fresh_score > stale_score,
        "fresh chunk must score higher than stale chunk on freshness: {fresh_score:.4} > {stale_score:.4}"
    );

    // End-to-end: confirm stale chunks rank lower in retrieval results.
    let store = Arc::new(InMemoryDocumentStore::new());
    let retrieval = InMemoryRetrieval::new(store.clone());

    let query_term = "distributed systems consensus raft";

    // Insert fresh chunk directly (created 1 hour ago).
    DocumentStore::insert_chunks(
        store.as_ref(),
        &[ChunkRecord {
            chunk_id: ChunkId::new("chunk_fresh"),
            document_id: doc("fresh_doc"),
            source_id: source("src_fresh"),
            source_type: SourceType::PlainText,
            project: project(),
            text: format!("{query_term} — recent coverage with the Raft algorithm."),
            position: 0,
            created_at: fresh_created_at,
            updated_at: None,
            provenance_metadata: None,
            credibility_score: None,
            graph_linkage: None,
            embedding: None,
            content_hash: Some(compute_content_hash(&format!("{query_term} fresh"))),
            entities: vec![],
        }],
    )
    .await
    .unwrap();

    // Insert stale chunk directly (created 120 days ago).
    DocumentStore::insert_chunks(
        store.as_ref(),
        &[ChunkRecord {
            chunk_id: ChunkId::new("chunk_stale"),
            document_id: doc("stale_doc"),
            source_id: source("src_stale"),
            source_type: SourceType::PlainText,
            project: project(),
            text: format!("{query_term} — older coverage of distributed consensus."),
            position: 0,
            created_at: stale_created_at,
            updated_at: None,
            provenance_metadata: None,
            credibility_score: None,
            graph_linkage: None,
            embedding: None,
            content_hash: Some(compute_content_hash(&format!("{query_term} stale"))),
            entities: vec![],
        }],
    )
    .await
    .unwrap();

    // Use an aggressive scoring policy that weights freshness heavily.
    let freshness_heavy_policy = ScoringPolicy {
        weights: ScoringWeights {
            freshness_weight: 0.5, // freshness dominates
            lexical_weight: 0.1,
            semantic_weight: 0.0,
            staleness_weight: 0.1,
            credibility_weight: 0.0,
            corroboration_weight: 0.0,
            graph_proximity_weight: 0.0,
            recency_weight: 0.0,
        },
        freshness_decay_days: decay_days,
        staleness_threshold_days: 90.0,
        recency_enabled: false,
        retrieval_mode_default: RetrievalMode::LexicalOnly,
        reranker_default: RerankerStrategy::None,
    };

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: query_term.to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: Some(freshness_heavy_policy),
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 2, "both chunks must be returned for the query");

    // Find the fresh and stale results by document ID.
    let fresh_result = response.results.iter().find(|r| r.chunk.document_id == doc("fresh_doc"))
        .expect("fresh document must appear in results");
    let stale_result = response.results.iter().find(|r| r.chunk.document_id == doc("stale_doc"))
        .expect("stale document must appear in results");

    // Fresh chunk must score significantly higher.
    assert!(
        fresh_result.breakdown.freshness > stale_result.breakdown.freshness,
        "fresh chunk freshness ({:.4}) must exceed stale chunk freshness ({:.4})",
        fresh_result.breakdown.freshness, stale_result.breakdown.freshness
    );
    assert!(
        fresh_result.score > stale_result.score,
        "fresh chunk final score ({:.4}) must exceed stale chunk score ({:.4}) \
         under freshness-heavy policy",
        fresh_result.score, stale_result.score
    );

    // Fresh result must appear first (sorted by descending score).
    assert_eq!(
        response.results[0].chunk.document_id,
        doc("fresh_doc"),
        "fresh document must rank first under freshness-heavy scoring policy"
    );
}
