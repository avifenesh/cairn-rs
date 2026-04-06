//! Composable service bundle for API/runtime wiring.
//!
//! Provides `MemoryServices` — a single injectable struct that bundles
//! all Worker 6 services. Worker 8 can construct this once at startup
//! and pass it to route handlers.

use std::sync::Arc;

use crate::api_impl::MemoryApiImpl;
use crate::deep_search::DeepSearchService;
use crate::deep_search_impl::IterativeDeepSearch;
use crate::diagnostics_impl::InMemoryDiagnostics;
use crate::feed_impl::FeedStore;
use crate::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use crate::ingest::IngestService;
use crate::pipeline::{IngestPipeline, ParagraphChunker};
use crate::retrieval::RetrievalService;

/// All Worker 6 services bundled for injection.
///
/// Worker 8 constructs this at startup and passes it to HTTP handlers.
///
/// ```ignore
/// let services = MemoryServices::in_memory();
/// // Wire into axum/actix router:
/// // app.route("/v1/memories/search", get(|q| services.memory.search(project, q)))
/// // app.route("/v1/feed", get(|q| services.feed.list(project, q)))
/// ```
pub struct MemoryServices<R: RetrievalService, I: IngestService, D: DeepSearchService> {
    pub memory: MemoryApiImpl<R>,
    pub feed: FeedStore,
    pub diagnostics: InMemoryDiagnostics,
    pub ingest: I,
    pub deep_search: D,
}

/// In-memory service bundle for local mode and testing.
pub type InMemoryServices = MemoryServices<
    InMemoryRetrieval,
    IngestPipeline<Arc<InMemoryDocumentStore>, ParagraphChunker>,
    IterativeDeepSearch<InMemoryRetrieval, crate::deep_search_impl::KeywordDecomposer>,
>;

impl InMemoryServices {
    /// Create a fully wired in-memory service bundle.
    ///
    /// All services share the same document store so ingest is
    /// immediately visible to retrieval and diagnostics.
    pub fn new() -> Self {
        let store = Arc::new(InMemoryDocumentStore::new());
        let chunker = ParagraphChunker::default();
        let pipeline = IngestPipeline::new(store.clone(), chunker);
        let retrieval = InMemoryRetrieval::new(store.clone());
        let deep_search = IterativeDeepSearch::new(InMemoryRetrieval::new(store.clone()));
        let memory_api = MemoryApiImpl::new(retrieval, store);
        let feed = FeedStore::new();
        let diagnostics = InMemoryDiagnostics::new();

        Self {
            memory: memory_api,
            feed,
            diagnostics,
            ingest: pipeline,
            deep_search,
        }
    }
}

impl Default for InMemoryServices {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_api::feed::FeedEndpoints;
    use cairn_api::memory_api::{MemoryEndpoints, MemorySearchQuery};
    use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};

    use crate::deep_search::DeepSearchRequest;
    use crate::ingest::{IngestRequest, SourceType};
    use crate::retrieval::RetrievalMode;

    #[tokio::test]
    async fn in_memory_services_wire_end_to_end() {
        let services = InMemoryServices::new();
        let project = ProjectKey::new("t", "w", "p");

        // Ingest a document.
        services
            .ingest
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_1"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: project.clone(),
                content: "Rust ownership and borrowing rules.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        // Search via memory API.
        let results = services
            .memory
            .search(
                &project,
                &MemorySearchQuery {
                    q: "ownership".to_owned(),
                    limit: Some(5),
                },
            )
            .await
            .unwrap();
        assert!(!results.is_empty());

        // Deep search.
        let deep = services
            .deep_search
            .search(DeepSearchRequest {
                project: project.clone(),
                query_text: "borrowing".to_owned(),
                max_hops: 2,
                per_hop_limit: 5,
                mode: RetrievalMode::LexicalOnly,
            })
            .await
            .unwrap();
        assert!(!deep.merged_results.is_empty());

        // Feed.
        use cairn_api::feed::{FeedItem, FeedQuery};
        services.feed.push_item(FeedItem {
            id: "f1".to_owned(),
            source: "rss".to_owned(),
            kind: None,
            title: Some("New".to_owned()),
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

        let feed = services
            .feed
            .list(&project, &FeedQuery::default())
            .await
            .unwrap();
        assert_eq!(feed.items.len(), 1);

        // Diagnostics.
        services
            .diagnostics
            .record_ingest(&SourceId::new("src"), &project, 3);
        let idx =
            crate::diagnostics::DiagnosticsService::index_status(&services.diagnostics, &project)
                .await
                .unwrap();
        assert_eq!(idx.total_chunks, 3);
    }
}
