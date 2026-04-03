//! API seam audit: proves MemoryEndpoints and FeedEndpoints calls
//! flow through real cairn-memory services, not route-local shaping.

use cairn_api::feed::{FeedEndpoints, FeedItem, FeedQuery};
use cairn_api::memory_api::{MemoryEndpoints, MemorySearchQuery};
use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::api_impl::MemoryApiImpl;
use cairn_memory::feed_impl::FeedStore;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use std::sync::Arc;

/// Proves MemoryEndpoints::search goes through RetrievalService,
/// not a route-local stub. Ingesting a doc then searching for it
/// must return results — which only works if search delegates to
/// the real retrieval backend sharing the same document store.
#[tokio::test]
async fn memory_search_flows_through_real_retrieval_service() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let project = ProjectKey::new("t", "w", "p");

    // Ingest through the pipeline.
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("audit_doc"),
            source_id: SourceId::new("audit_src"),
            source_type: SourceType::PlainText,
            project: project.clone(),
            content: "Kubernetes pod scheduling uses node affinity rules.".to_owned(),
        })
        .await
        .unwrap();

    // Build MemoryApiImpl with same backing store.
    let retrieval = InMemoryRetrieval::new(store);
    let api = MemoryApiImpl::new(retrieval);

    // Search through the API trait — must find the doc.
    let results = api
        .search(
            &project,
            &MemorySearchQuery {
                q: "kubernetes pod scheduling".to_owned(),
                limit: Some(5),
            },
        )
        .await
        .unwrap();

    assert!(!results.is_empty(), "search must find ingested content");
    assert!(
        results[0].content.to_lowercase().contains("kubernetes"),
        "result must come from the real retrieval backend"
    );
}

/// Proves FeedEndpoints::list goes through real FeedStore state,
/// not a route-local stub. Pushing an item then listing must return
/// it — which only works if list reads from the real shared store.
#[tokio::test]
async fn feed_list_flows_through_real_feed_store() {
    let feed = FeedStore::new();
    let project = ProjectKey::new("t", "w", "p");

    // Push through the store directly (simulating a signal poller).
    feed.push_item(FeedItem {
        id: "audit_feed_1".to_owned(),
        source: "audit_source".to_owned(),
        kind: None,
        title: Some("Audit item".to_owned()),
        body: None,
        url: None,
        author: None,
        avatar_url: None,
        repo_full_name: None,
        is_read: false,
        is_archived: false,
        group_key: None,
        created_at: "2026-04-03T09:30:00Z".to_owned(),
    });

    // List through the API trait — must find the item.
    let result = feed.list(&project, &FeedQuery::default()).await.unwrap();

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].id, "audit_feed_1");
    assert_eq!(result.items[0].source, "audit_source");
}
