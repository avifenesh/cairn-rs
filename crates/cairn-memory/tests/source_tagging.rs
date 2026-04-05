//! RFC 003 source tagging and filtering integration tests.

use std::sync::Arc;

use cairn_api::memory_api::{AddSourceTagsRequest, SourceTagsEndpoints};
use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::api_impl::SourceTagsApiImpl;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{MetadataFilter, RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// Helper: check if a chunk's provenance_metadata contains a given tag.
fn chunk_has_tag(chunk: &cairn_memory::ingest::ChunkRecord, tag: &str) -> bool {
    chunk
        .provenance_metadata
        .as_ref()
        .and_then(|m| m.get("tags"))
        .and_then(|v| v.as_array())
        .map_or(false, |arr| arr.iter().any(|v| v.as_str() == Some(tag)))
}

/// Ingest doc with tags=['production','qa'].
/// Search with tag='production' → result returned.
/// Search with tag='staging' → no result.
#[tokio::test]
async fn source_tagging_filter_by_tag_returns_matching_chunks() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_tagged"),
            source_id: SourceId::new("src_prod"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Production deployment guide for widgets and services.".to_owned(),
            tags: vec!["production".to_owned(), "qa".to_owned()],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // Search with tag='production' — must return the document.
    let prod_response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "deployment guide".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![MetadataFilter {
                key: "tag".to_owned(),
                value: "production".to_owned(),
            }],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(
        !prod_response.results.is_empty(),
        "should find chunks tagged 'production'"
    );
    for result in &prod_response.results {
        assert!(
            chunk_has_tag(&result.chunk, "production"),
            "returned chunk must have 'production' tag"
        );
    }

    // Search with tag='staging' — must return nothing.
    let staging_response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "deployment guide".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![MetadataFilter {
                key: "tag".to_owned(),
                value: "staging".to_owned(),
            }],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(
        staging_response.results.is_empty(),
        "should find no chunks tagged 'staging'"
    );
}

/// Tags on IngestRequest propagate to all chunks produced from that document.
#[tokio::test]
async fn source_tagging_tags_propagate_to_all_chunks() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(
        store.clone(),
        ParagraphChunker { max_chunk_size: 30 },
    );

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_multi_chunk"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "First paragraph about systems.\n\nSecond paragraph about services.\n\nThird paragraph about reliability.".to_owned(),
            tags: vec!["infra".to_owned()],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let chunks = store.all_current_chunks();
    assert!(chunks.len() >= 2, "expected multiple chunks");
    for chunk in &chunks {
        assert!(
            chunk_has_tag(chunk, "infra"),
            "every chunk must carry the 'infra' tag (chunk_id={})",
            chunk.chunk_id
        );
    }
}

/// POST /v1/sources/:id/tags propagates tags to all existing chunks from that source.
/// GET /v1/sources/:id/tags returns the current tag list.
#[tokio::test]
async fn source_tagging_api_add_and_get_source_tags() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let api = SourceTagsApiImpl::new(store.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_src_tags"),
            source_id: SourceId::new("src_ops"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Ops runbook: restart the service when alerts fire.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // Before tagging — GET returns empty list.
    let before = api.get_source_tags("src_ops").await.unwrap();
    assert!(before.tags.is_empty(), "source should have no tags initially");

    // POST tags to the source.
    let added = api
        .add_source_tags(
            "src_ops",
            &AddSourceTagsRequest {
                tags: vec!["ops".to_owned(), "critical".to_owned()],
            },
        )
        .await
        .unwrap();
    assert!(added.tags.contains(&"ops".to_owned()));
    assert!(added.tags.contains(&"critical".to_owned()));

    // GET now returns the new tags.
    let after = api.get_source_tags("src_ops").await.unwrap();
    assert_eq!(after.source_id, "src_ops");
    assert!(after.tags.contains(&"ops".to_owned()));
    assert!(after.tags.contains(&"critical".to_owned()));

    // Chunks from that source now carry the retroactively-assigned tags.
    let chunks = store.all_current_chunks();
    let src_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.source_id == SourceId::new("src_ops"))
        .collect();
    assert!(!src_chunks.is_empty());
    for chunk in src_chunks {
        assert!(
            chunk_has_tag(chunk, "ops"),
            "chunk must carry the newly-added 'ops' tag"
        );
    }
}

/// POST tags is idempotent — adding the same tag twice does not duplicate it.
#[tokio::test]
async fn source_tagging_add_tags_is_idempotent() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let api = SourceTagsApiImpl::new(store.clone());

    api.add_source_tags(
        "src_idem",
        &AddSourceTagsRequest {
            tags: vec!["env:prod".to_owned()],
        },
    )
    .await
    .unwrap();

    api.add_source_tags(
        "src_idem",
        &AddSourceTagsRequest {
            tags: vec!["env:prod".to_owned()],
        },
    )
    .await
    .unwrap();

    let result = api.get_source_tags("src_idem").await.unwrap();
    assert_eq!(
        result.tags.iter().filter(|t| *t == "env:prod").count(),
        1,
        "duplicate tags must be deduplicated"
    );
}
