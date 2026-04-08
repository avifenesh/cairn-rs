//! Integration test: full ingest-to-retrieval pipeline with normalization,
//! scoring, and diagnostics (RFC 003 Phase 4 proof).

use std::sync::Arc;

use cairn_domain::*;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{
    CandidateStage, MetadataFilter, RerankerStrategy, RetrievalMode, RetrievalQuery,
    RetrievalService,
};

fn project() -> ProjectKey {
    ProjectKey::new("acme", "eng", "docs")
}

async fn setup() -> (
    IngestPipeline<Arc<InMemoryDocumentStore>, ParagraphChunker>,
    InMemoryRetrieval,
) {
    let store = Arc::new(InMemoryDocumentStore::new());
    let chunker = ParagraphChunker {
        max_chunk_size: 300,
    };
    let pipeline = IngestPipeline::new(store.clone(), chunker);
    let retrieval = InMemoryRetrieval::new(store);

    // Ingest an HTML document.
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_html_guide"),
            source_id: SourceId::new("wiki"),
            source_type: SourceType::Html,
            project: project(),
            content:
                "<h1>Deployment Guide</h1>\
                      <p>Step 1: Build the container image using <code>docker build</code>.</p>\
                      <p>Step 2: Push to the registry with <code>docker push</code>.</p>\
                      <p>Step 3: Apply the Kubernetes manifest with <code>kubectl apply</code>.</p>"
                    .to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // Ingest a Markdown document.
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_md_troubleshoot"),
            source_id: SourceId::new("wiki"),
            source_type: SourceType::Markdown,
            project: project(),
            content: "# Troubleshooting\n\n\
                      ## Container build failures\n\n\
                      If `docker build` fails, check the Dockerfile syntax and base image availability.\n\n\
                      ## Kubernetes pod crashes\n\n\
                      Use `kubectl logs` and `kubectl describe pod` to diagnose crash loops."
                .to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    (pipeline, retrieval)
}

#[tokio::test]
async fn hybrid_query_returns_scored_results_with_diagnostics() {
    let (_pipeline, retrieval) = setup().await;

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "docker build container".to_owned(),
            mode: RetrievalMode::Hybrid,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    // Both documents mention docker build — should have results.
    assert!(
        response.results.len() >= 2,
        "expected results from both docs, got {}",
        response.results.len()
    );

    // Every result has a populated scoring breakdown.
    for result in &response.results {
        assert!(
            result.breakdown.lexical_relevance > 0.0,
            "lexical_relevance should be positive for matching chunks"
        );
        assert!(
            result.breakdown.freshness > 0.0,
            "freshness should be positive for recently-created chunks"
        );
        assert!(
            result.score > 0.0,
            "final weighted score should be positive"
        );
    }

    // Diagnostics: stages_used and scoring_dimensions_used populated.
    let diag = &response.diagnostics;
    assert!(
        !diag.stages_used.is_empty(),
        "stages_used must not be empty"
    );
    assert!(
        diag.stages_used.contains(&CandidateStage::Lexical),
        "Lexical stage should be listed"
    );
    assert!(
        diag.scoring_dimensions_used
            .contains(&"lexical_relevance".to_owned()),
        "lexical_relevance must be in scoring_dimensions_used"
    );
    assert!(
        diag.scoring_dimensions_used
            .contains(&"freshness".to_owned()),
        "freshness must be in scoring_dimensions_used"
    );
    assert!(
        diag.effective_policy.is_some(),
        "effective_policy should describe the policy used"
    );

    // Hybrid falls back to LexicalOnly in the in-memory backend.
    assert_eq!(diag.mode_used, RetrievalMode::LexicalOnly);
    assert!(diag.candidates_generated > 0);
    assert!(diag.results_returned > 0);
}

#[tokio::test]
async fn html_content_is_normalized_before_chunking() {
    let (_pipeline, retrieval) = setup().await;

    // Search for content that only exists after HTML stripping.
    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Deployment Guide".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(!response.results.is_empty(), "should find the guide");

    // Chunks should not contain raw HTML tags.
    for result in &response.results {
        assert!(
            !result.chunk.text.contains("<h1>"),
            "chunk should not contain raw HTML tags"
        );
        assert!(
            !result.chunk.text.contains("<p>"),
            "chunk should not contain <p> tags"
        );
    }
}

#[tokio::test]
async fn metadata_filter_restricts_to_html_source_type() {
    let (_pipeline, retrieval) = setup().await;

    // Without filter: both docs match "docker".
    let all = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "docker".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    let doc_ids: Vec<String> = all
        .results
        .iter()
        .map(|r| r.chunk.document_id.to_string())
        .collect();
    assert!(
        doc_ids.contains(&"doc_html_guide".to_owned()),
        "HTML doc should match"
    );
    assert!(
        doc_ids.contains(&"doc_md_troubleshoot".to_owned()),
        "Markdown doc should match"
    );

    // With filter: only Html source type.
    let filtered = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "docker".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![MetadataFilter {
                key: "source_type".to_owned(),
                value: "Html".to_owned(),
            }],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(!filtered.results.is_empty(), "should find Html results");
    for result in &filtered.results {
        assert_eq!(
            result.chunk.source_type,
            SourceType::Html,
            "all results should be Html source type"
        );
    }
}
