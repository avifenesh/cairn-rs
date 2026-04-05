//! RFC 003 memory document lifecycle integration tests.
//!
//! Validates the full knowledge pipeline:
//! - Ingest a document with chunks via IngestPipeline.
//! - Retrieve the document through InMemoryRetrieval.
//! - Update document with new content and verify new chunks land in the store.
//! - Source quality tracking via InMemoryDiagnostics.
//! - Project-scoped retrieval: queries must not leak across project boundaries.

use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::diagnostics::DiagnosticsService;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, IngestStatus, SourceType};
use cairn_memory::pipeline::{DocumentStore, IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};
use cairn_memory::diagnostics_impl::InMemoryDiagnostics;

// ── helpers ───────────────────────────────────────────────────────────────────

fn project_a() -> ProjectKey {
    ProjectKey::new("tenant_mem", "ws_mem", "proj_a")
}

fn project_b() -> ProjectKey {
    ProjectKey::new("tenant_mem", "ws_mem", "proj_b")
}

fn doc_id(name: &str) -> KnowledgeDocumentId {
    KnowledgeDocumentId::new(name)
}

fn source_id() -> SourceId {
    SourceId::new("src_docs")
}

fn ingest_req(
    id: &str,
    project: ProjectKey,
    content: &str,
) -> IngestRequest {
    IngestRequest {
        document_id: doc_id(id),
        source_id: source_id(),
        source_type: SourceType::PlainText,
        project,
        content: content.to_owned(),
        tags: vec![],
        corpus_id: None,
        bundle_source_id: None,
        import_id: None,
    }
}

fn query(project: ProjectKey, text: &str) -> RetrievalQuery {
    RetrievalQuery {
        project,
        query_text: text.to_owned(),
        mode: RetrievalMode::Hybrid,
        reranker: RerankerStrategy::None,
        limit: 10,
        metadata_filters: vec![],
        scoring_policy: None,
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) Create InMemoryDocumentStore; (2) ingest a document with chunks;
/// (3) verify retrieval returns the document.
#[tokio::test]
async fn ingest_and_retrieve_document() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    // Step 1 + 2: ingest a document.
    pipeline
        .submit(ingest_req(
            "doc_intro",
            project_a(),
            "Rust is a systems programming language focused on safety and performance. \
             It uses ownership to manage memory without a garbage collector. \
             The borrow checker prevents data races at compile time.",
        ))
        .await
        .unwrap();

    // Verify status is Completed.
    let status = DocumentStore::get_status(store.as_ref(), &doc_id("doc_intro")).await.unwrap();
    assert_eq!(
        status,
        Some(IngestStatus::Completed),
        "document status must be Completed after successful ingest"
    );

    // Verify chunks were created.
    let chunks = store.all_chunks();
    assert!(!chunks.is_empty(), "ingest must produce at least one chunk");
    let doc_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.document_id == doc_id("doc_intro"))
        .collect();
    assert!(!doc_chunks.is_empty(), "chunks must be associated with the document ID");
    assert!(
        doc_chunks.iter().all(|c| c.project == project_a()),
        "all chunks must carry the correct project key"
    );

    // Step 3: retrieve the document via a relevant query.
    let response = retrieval
        .query(query(project_a(), "Rust memory safety borrow checker"))
        .await
        .unwrap();

    assert!(
        !response.results.is_empty(),
        "retrieval must return results for a query matching the ingested content"
    );
    // At least one result must come from our document.
    let has_doc = response
        .results
        .iter()
        .any(|r| r.chunk.document_id == doc_id("doc_intro"));
    assert!(has_doc, "retrieval results must include the ingested document");

    // Diagnostics must be present.
    assert!(
        response.diagnostics.results_returned > 0,
        "retrieval diagnostics must report non-zero results_returned"
    );
    assert!(
        !response.diagnostics.stages_used.is_empty(),
        "diagnostics must list the candidate generation stages used"
    );
}

/// (4) Update document with new content; (5) verify new chunks are added
/// (the re-ingest appends chunks with the new content hash).
#[tokio::test]
async fn update_document_adds_new_chunks() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    let v1_content =
        "Version one: This document covers the introduction to async Rust programming.";
    let v2_content =
        "Version two: This document covers advanced async patterns including futures and executors.";

    // Ingest version 1.
    pipeline
        .submit(ingest_req("doc_versioned", project_a(), v1_content))
        .await
        .unwrap();

    let chunks_after_v1 = store
        .all_chunks()
        .into_iter()
        .filter(|c| c.document_id == doc_id("doc_versioned"))
        .count();
    assert!(chunks_after_v1 > 0, "version 1 must produce chunks");

    // Ingest version 2 (different content hash → not deduped).
    pipeline
        .submit(ingest_req("doc_versioned", project_a(), v2_content))
        .await
        .unwrap();

    let chunks_after_v2 = store
        .all_chunks()
        .into_iter()
        .filter(|c| c.document_id == doc_id("doc_versioned"))
        .count();

    // New content hash means new chunks are added alongside the old ones.
    assert!(
        chunks_after_v2 > chunks_after_v1,
        "re-ingesting with different content must add new chunks (got {chunks_after_v1} → {chunks_after_v2})"
    );

    // Document status remains Completed after update.
    let status = DocumentStore::get_status(store.as_ref(), &doc_id("doc_versioned")).await.unwrap();
    assert_eq!(status, Some(IngestStatus::Completed));

    // The new content hash differs from the old one.
    let all_chunks = store.all_chunks();
    let hashes: std::collections::HashSet<_> = all_chunks
        .iter()
        .filter(|c| c.document_id == doc_id("doc_versioned"))
        .filter_map(|c| c.content_hash.as_ref())
        .cloned()
        .collect();
    assert!(
        hashes.len() > 1,
        "v1 and v2 must produce distinct content hashes"
    );
}

/// Deduplication: re-ingesting identical content does not add duplicate chunks.
#[tokio::test]
async fn identical_re_ingest_is_deduped() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    let content = "Identical content about idempotent ingest operations.";

    pipeline
        .submit(ingest_req("doc_dedup", project_a(), content))
        .await
        .unwrap();
    let count_v1 = store
        .all_chunks()
        .into_iter()
        .filter(|c| c.document_id == doc_id("doc_dedup"))
        .count();

    // Re-ingest same content.
    pipeline
        .submit(ingest_req("doc_dedup", project_a(), content))
        .await
        .unwrap();
    let count_v2 = store
        .all_chunks()
        .into_iter()
        .filter(|c| c.document_id == doc_id("doc_dedup"))
        .count();

    assert_eq!(
        count_v1, count_v2,
        "re-ingesting identical content must not add duplicate chunks (dedup by hash)"
    );
}

/// (6) Source quality tracking: InMemoryDiagnostics records ingest events and
/// tracks retrieval hits, exposing source quality and index status.
#[tokio::test]
async fn source_quality_tracking_via_diagnostics() {
    let diag = InMemoryDiagnostics::new();

    // Record two ingest events for different sources.
    diag.record_ingest(&source_id(), &project_a(), 5);
    diag.record_ingest(&SourceId::new("src_secondary"), &project_a(), 3);

    // Source quality must reflect the chunk counts.
    let quality = diag
        .source_quality(&source_id())
        .await
        .unwrap()
        .expect("source quality must exist after record_ingest");
    assert_eq!(quality.total_chunks, 5, "chunk count must match ingested count");
    assert_eq!(quality.total_retrievals, 0, "no retrievals yet");

    // Record retrieval hits.
    diag.record_retrieval_hit(&source_id(), 0.9);
    diag.record_retrieval_hit(&source_id(), 0.7);

    let quality_after = diag
        .source_quality(&source_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(quality_after.total_retrievals, 2, "retrieval count must update");
    assert!(
        (quality_after.avg_relevance_score - 0.8).abs() < 0.01,
        "avg_relevance_score must be mean of 0.9 and 0.7 = 0.8 (got {})",
        quality_after.avg_relevance_score
    );

    // Index status must aggregate across both sources for the project.
    let idx = diag.index_status(&project_a()).await.unwrap();
    assert_eq!(idx.total_documents, 2, "index must track 2 documents");
    assert_eq!(idx.total_chunks, 8, "index must sum chunks across sources (5+3)");

    // list_source_quality returns both sources.
    let all_quality = diag.list_source_quality(&project_a(), 10).await.unwrap();
    assert_eq!(all_quality.len(), 2, "both sources must appear in quality list");
}

/// (7) Search by project returns only project-scoped documents — queries must
/// not leak results across project boundaries.
#[tokio::test]
async fn retrieval_is_scoped_to_project() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    // Ingest the same topic into two different projects.
    let shared_query = "database indexing and query optimization";

    pipeline
        .submit(ingest_req(
            "doc_proj_a",
            project_a(),
            "Project A: database indexing strategies improve query optimization performance.",
        ))
        .await
        .unwrap();

    pipeline
        .submit(ingest_req(
            "doc_proj_b",
            project_b(),
            "Project B: query optimization relies on proper database indexing and statistics.",
        ))
        .await
        .unwrap();

    // Query project A — must only return project A documents.
    let resp_a = retrieval
        .query(query(project_a(), shared_query))
        .await
        .unwrap();

    let proj_a_leak = resp_a
        .results
        .iter()
        .any(|r| r.chunk.project == project_b());
    assert!(
        !proj_a_leak,
        "project A query must not return results from project B"
    );
    assert!(
        !resp_a.results.is_empty(),
        "project A query must return its own document"
    );

    // Query project B — must only return project B documents.
    let resp_b = retrieval
        .query(query(project_b(), shared_query))
        .await
        .unwrap();

    let proj_b_leak = resp_b
        .results
        .iter()
        .any(|r| r.chunk.project == project_a());
    assert!(
        !proj_b_leak,
        "project B query must not return results from project A"
    );
    assert!(
        !resp_b.results.is_empty(),
        "project B query must return its own document"
    );

    // Cross-check: the document IDs are project-specific.
    let a_doc_ids: std::collections::HashSet<_> = resp_a
        .results
        .iter()
        .map(|r| r.chunk.document_id.as_str())
        .collect();
    let b_doc_ids: std::collections::HashSet<_> = resp_b
        .results
        .iter()
        .map(|r| r.chunk.document_id.as_str())
        .collect();
    assert!(
        a_doc_ids.is_disjoint(&b_doc_ids),
        "project A and project B result sets must not overlap"
    );
}

/// Multiple documents in the same project are all retrievable and correctly attributed.
#[tokio::test]
async fn multiple_documents_in_project_are_all_retrievable() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    pipeline
        .submit(ingest_req(
            "doc_alpha",
            project_a(),
            "Alpha document: async runtime and tokio executor internals.",
        ))
        .await
        .unwrap();

    pipeline
        .submit(ingest_req(
            "doc_beta",
            project_a(),
            "Beta document: synchronous blocking I/O and thread pool management.",
        ))
        .await
        .unwrap();

    // Each document must be independently retrievable.
    let async_results = retrieval
        .query(query(project_a(), "tokio async runtime"))
        .await
        .unwrap();
    let has_alpha = async_results
        .results
        .iter()
        .any(|r| r.chunk.document_id == doc_id("doc_alpha"));
    assert!(has_alpha, "query about async runtime must hit doc_alpha");

    let sync_results = retrieval
        .query(query(project_a(), "blocking thread pool"))
        .await
        .unwrap();
    let has_beta = sync_results
        .results
        .iter()
        .any(|r| r.chunk.document_id == doc_id("doc_beta"));
    assert!(has_beta, "query about blocking I/O must hit doc_beta");
}
