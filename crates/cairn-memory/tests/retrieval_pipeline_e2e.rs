//! RFC 003 — memory retrieval pipeline end-to-end integration tests.
//!
//! Covers:
//!   1. Multi-document ingest
//!   2. LexicalOnly query ranked by relevance
//!   3. Metadata filter via tag restricts results
//!   4. Dedup:
//!      - Within-document dedup (duplicate paragraph stored once)
//!      - Cross-submit dedup (same content, different doc ID)
//!   5. Source quality tracking via chunk counts after ingestion

use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{
    MetadataFilter, RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService,
};

fn project() -> ProjectKey {
    ProjectKey::new("tenant_rfc003", "ws_rfc003", "proj_rfc003")
}

fn plain_req(doc_id: &str, source_id: &str, content: &str) -> IngestRequest {
    IngestRequest {
        document_id: KnowledgeDocumentId::new(doc_id),
        source_id: SourceId::new(source_id),
        source_type: SourceType::PlainText,
        project: project(),
        content: content.to_owned(),
        import_id: None,
        corpus_id: None,
        bundle_source_id: None,
        tags: vec![],
    }
}

fn tagged_req(doc_id: &str, source_id: &str, content: &str, tags: Vec<&str>) -> IngestRequest {
    IngestRequest {
        document_id: KnowledgeDocumentId::new(doc_id),
        source_id: SourceId::new(source_id),
        source_type: SourceType::PlainText,
        project: project(),
        content: content.to_owned(),
        import_id: None,
        corpus_id: None,
        bundle_source_id: None,
        tags: tags.into_iter().map(str::to_owned).collect(),
    }
}

// ── Test 1 + 2: Ingest three documents, verify LexicalOnly ranks by relevance ──

/// RFC 003 §5: lexical retrieval must rank results by query-term overlap.
///
/// Three documents are ingested with clearly differentiated subject matter.
/// A query targeting machine-learning vocabulary must surface the ML document
/// first; unrelated documents (biology, history) must rank lower.
#[tokio::test]
async fn lexical_query_ranks_most_relevant_document_first() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    // doc_bio: plant biology — zero overlap with ML vocabulary.
    pipeline
        .submit(plain_req(
            "doc_bio",
            "src_bio",
            "Photosynthesis is the process by which plants use sunlight to produce energy.\n\n\
             Chlorophyll is the pigment that gives plants their distinctive green colour.\n\n\
             Plants convert carbon dioxide and water into glucose through photosynthesis.",
        ))
        .await
        .unwrap();

    // doc_ml: machine learning — high overlap with query terms.
    pipeline
        .submit(plain_req(
            "doc_ml",
            "src_ml",
            "Machine learning algorithms learn from training data by minimising a loss function.\n\n\
             Gradient descent is the core optimisation algorithm used in machine learning to \
             iteratively update model weights toward lower loss.\n\n\
             Deep learning extends machine learning with multi-layer neural network architectures.",
        ))
        .await
        .unwrap();

    // doc_hist: Roman history — zero overlap with ML vocabulary.
    pipeline
        .submit(plain_req(
            "doc_hist",
            "src_hist",
            "The Roman Empire reached its greatest territorial extent under Emperor Trajan.\n\n\
             Roman roads connected the provinces to the capital and enabled rapid troop movement.\n\n\
             The Western Roman Empire fell in 476 AD when Romulus Augustulus was deposed.",
        ))
        .await
        .unwrap();

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "machine learning gradient descent".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(
        !response.results.is_empty(),
        "LexicalOnly query must return at least one result"
    );

    // The top-ranked result must come from the ML document.
    let top_doc = response.results[0].chunk.document_id.as_str();
    assert_eq!(
        top_doc, "doc_ml",
        "doc_ml must rank first for 'machine learning gradient descent'; got: {top_doc}"
    );

    // All results must belong to this project.
    for r in &response.results {
        assert_eq!(
            r.chunk.project,
            project(),
            "every result must belong to the test project"
        );
    }

    // Scores must be sorted in descending order.
    let scores: Vec<f64> = response.results.iter().map(|r| r.score).collect();
    for window in scores.windows(2) {
        assert!(
            window[0] >= window[1],
            "results must be sorted by descending score; scores: {scores:?}"
        );
    }

    // Diagnostics must report LexicalOnly was used.
    assert_eq!(
        response.diagnostics.mode_used,
        RetrievalMode::LexicalOnly,
        "diagnostics must confirm LexicalOnly mode was used"
    );
    assert_eq!(
        response.diagnostics.results_returned,
        response.results.len(),
        "diagnostics results_returned must match actual result count"
    );
}

// ── Test 3: Metadata tag filter restricts results to matching chunks only ──

/// RFC 003 §6: metadata filters must exclude chunks that do not match.
///
/// Two documents are ingested: one tagged "science", one untagged.
/// A query with a tag filter for "science" must return only the tagged
/// document's chunks; the untagged document must not appear.
#[tokio::test]
async fn metadata_tag_filter_excludes_untagged_chunks() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    // doc_science: tagged "science" — should survive the filter.
    pipeline
        .submit(tagged_req(
            "doc_science",
            "src_sci",
            "Quantum mechanics describes the behaviour of subatomic particles.\n\n\
             The Heisenberg uncertainty principle states that position and momentum \
             cannot both be known precisely at the same time.",
            vec!["science"],
        ))
        .await
        .unwrap();

    // doc_news: untagged — must be excluded by the "science" filter.
    pipeline
        .submit(plain_req(
            "doc_news",
            "src_news",
            "Quantum computing investment reached record levels this quarter.\n\n\
             The Heisenberg uncertainty about market timing continues to concern analysts.",
        ))
        .await
        .unwrap();

    // Without filter: both documents should contribute chunks.
    let unfiltered = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "quantum heisenberg".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 20,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    let unfiltered_docs: std::collections::HashSet<&str> = unfiltered
        .results
        .iter()
        .map(|r| r.chunk.document_id.as_str())
        .collect();
    assert!(
        unfiltered_docs.contains("doc_science"),
        "unfiltered query must include doc_science chunks"
    );
    assert!(
        unfiltered_docs.contains("doc_news"),
        "unfiltered query must include doc_news chunks (no filter applied)"
    );

    // With tag filter "science": only doc_science chunks must appear.
    let filtered = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "quantum heisenberg".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 20,
            metadata_filters: vec![MetadataFilter {
                key: "tag".to_owned(),
                value: "science".to_owned(),
            }],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(
        !filtered.results.is_empty(),
        "tag=science filter must return doc_science chunks"
    );
    for r in &filtered.results {
        assert_eq!(
            r.chunk.document_id.as_str(),
            "doc_science",
            "every filtered result must come from doc_science; got: {}",
            r.chunk.document_id.as_str()
        );
    }
}

// ── Test 4a: Within-document dedup — duplicate paragraphs stored once ──

/// RFC 003 §4: within a single ingest call, duplicate paragraphs must be
/// stored exactly once (within-batch dedup via content_hash).
///
/// `max_chunk_size: 30` forces a split at every blank line because each
/// paragraph exceeds 30 bytes; the duplicate paragraph's hash is caught by
/// the within-batch `batch_seen` set before it can be stored.
#[tokio::test]
async fn within_document_duplicate_paragraph_stored_once() {
    let store = Arc::new(InMemoryDocumentStore::new());
    // Small max_chunk_size so each paragraph becomes its own chunk,
    // making within-batch content_hash dedup observable.
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker { max_chunk_size: 30 });

    // Paragraph A appears at position 0 and again at position 2.
    let repeated = "All models are wrong but some are useful.";
    let content = format!(
        "{repeated}\n\n\
         This middle paragraph is unique and appears only once in the document.\n\n\
         {repeated}"
    );

    pipeline
        .submit(plain_req("doc_within_dedup", "src_dedup_a", &content))
        .await
        .unwrap();

    let chunks = store.all_chunks();
    let repeated_count = chunks
        .iter()
        .filter(|c| c.text.contains("All models are wrong"))
        .count();

    assert_eq!(
        repeated_count, 1,
        "RFC 003: duplicate paragraph must appear exactly once after within-batch dedup; \
         found {repeated_count} copies"
    );

    // The unique paragraph should still be present.
    let unique_present = chunks
        .iter()
        .any(|c| c.text.contains("middle paragraph is unique"));
    assert!(
        unique_present,
        "the non-duplicate paragraph must still be stored"
    );
}

// ── Test 4b: Cross-submit dedup — same content, different doc ID ──

/// RFC 003 §4: re-ingesting the same content under a different document ID
/// must not produce additional chunks (cross-batch content_hash dedup).
#[tokio::test]
async fn cross_submit_dedup_prevents_duplicate_storage() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    let content =
        "Deduplication ensures identical content is never stored twice in the knowledge base.\n\n\
         This property is critical for both storage efficiency and retrieval quality scoring.";

    // First submit.
    pipeline
        .submit(plain_req("doc_dedup_first", "src_dedup_b", content))
        .await
        .unwrap();

    let after_first = store.all_chunks().len();
    assert!(
        after_first > 0,
        "first submit must produce at least one chunk"
    );

    // Second submit: identical content, different document ID.
    pipeline
        .submit(plain_req("doc_dedup_second", "src_dedup_b", content))
        .await
        .unwrap();

    let after_second = store.all_chunks().len();
    assert_eq!(
        after_first, after_second,
        "RFC 003: re-ingesting identical content must not add new chunks; \
         before={after_first}, after={after_second}"
    );
}

// ── Test 5: Source quality tracking — chunk counts per document ──

/// RFC 003 §7: the store must faithfully track how many chunks each source
/// contributed. Chunk counts after ingestion must match expected paragraph counts.
///
/// `max_chunk_size: 30` forces one chunk per paragraph because every paragraph
/// is > 30 bytes; this makes the per-document chunk count directly observable.
#[tokio::test]
async fn source_quality_tracking_chunk_counts_match_paragraphs() {
    let store = Arc::new(InMemoryDocumentStore::new());
    // Small max_chunk_size so each paragraph is emitted as its own chunk.
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker { max_chunk_size: 30 });

    // Three documents with known, distinct paragraph counts.
    pipeline
        .submit(plain_req(
            "doc_track_three",
            "src_track_a",
            "Tracking paragraph alpha — first distinct section.\n\n\
             Tracking paragraph beta — second distinct section.\n\n\
             Tracking paragraph gamma — third distinct section.",
        ))
        .await
        .unwrap();

    pipeline
        .submit(plain_req(
            "doc_track_one",
            "src_track_b",
            "Single tracking paragraph with no separators and unique sentinel text xyzq1.",
        ))
        .await
        .unwrap();

    pipeline
        .submit(plain_req(
            "doc_track_four",
            "src_track_c",
            "Tracking section one — unique sentinel xyzq2.\n\n\
             Tracking section two — unique sentinel xyzq3.\n\n\
             Tracking section three — unique sentinel xyzq4.\n\n\
             Tracking section four — unique sentinel xyzq5.",
        ))
        .await
        .unwrap();

    let all = store.all_chunks();

    // All chunks must have non-empty text and a content_hash.
    for chunk in &all {
        assert!(
            !chunk.text.trim().is_empty(),
            "no stored chunk may have empty text"
        );
        assert!(
            chunk.content_hash.is_some(),
            "every chunk must carry a content_hash for dedup; chunk_id={}",
            chunk.chunk_id.as_str()
        );
        assert_eq!(
            chunk.project,
            project(),
            "all chunks must be scoped to the test project"
        );
    }

    // doc_track_three: 3 paragraphs → 3 chunks.
    let three_chunks: Vec<_> = all
        .iter()
        .filter(|c| c.document_id.as_str() == "doc_track_three")
        .collect();
    assert_eq!(
        three_chunks.len(),
        3,
        "doc_track_three (3 paragraphs) must produce 3 chunks; got {}",
        three_chunks.len()
    );

    // doc_track_one: 1 paragraph → 1 chunk.
    let one_chunks: Vec<_> = all
        .iter()
        .filter(|c| c.document_id.as_str() == "doc_track_one")
        .collect();
    assert_eq!(
        one_chunks.len(),
        1,
        "doc_track_one (1 paragraph) must produce 1 chunk; got {}",
        one_chunks.len()
    );

    // doc_track_four: 4 paragraphs → 4 chunks.
    let four_chunks: Vec<_> = all
        .iter()
        .filter(|c| c.document_id.as_str() == "doc_track_four")
        .collect();
    assert_eq!(
        four_chunks.len(),
        4,
        "doc_track_four (4 paragraphs) must produce 4 chunks; got {}",
        four_chunks.len()
    );

    // Grand total: 3 + 1 + 4 = 8.
    assert_eq!(
        all.len(),
        8,
        "grand total must equal 3 + 1 + 4 = 8; got {}",
        all.len()
    );

    // Each chunk's position should be monotonically increasing within its document.
    for doc_id in ["doc_track_three", "doc_track_one", "doc_track_four"] {
        let mut positions: Vec<u32> = all
            .iter()
            .filter(|c| c.document_id.as_str() == doc_id)
            .map(|c| c.position)
            .collect();
        positions.sort_unstable();
        let is_sequential = positions
            .iter()
            .enumerate()
            .all(|(i, &pos)| pos == i as u32);
        assert!(
            is_sequential,
            "chunk positions for {doc_id} must be sequential starting at 0; got {positions:?}"
        );
    }
}
