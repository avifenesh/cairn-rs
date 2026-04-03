//! Implementation of Worker 8's MemoryEndpoints backed by cairn-memory services.
//!
//! Wires the API-facing memory CRUD and search to the owned retrieval
//! pipeline, closing the seam between cairn-api and cairn-memory.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_api::endpoints::ListQuery;
use cairn_api::http::ListResponse;
use cairn_api::memory_api::{
    CreateMemoryRequest, MemoryEndpoints, MemoryItem, MemorySearchQuery, MemoryStatus,
};
use cairn_domain::ProjectKey;

use crate::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};

/// Memory endpoint implementation backed by cairn-memory services.
///
/// Search delegates to `RetrievalService`. CRUD operations use a
/// simple in-memory store for now -- will be backed by the document
/// store when memory entities are wired to the canonical store.
/// Callback for memory proposal events.
/// Worker 8's SSE publisher implements this to emit `memory_proposed` frames.
pub trait MemoryProposalHook: Send + Sync {
    fn on_proposed(&self, item: &MemoryItem);
}

/// No-op hook for tests and backends that don't need SSE.
pub struct NoOpProposalHook;
impl MemoryProposalHook for NoOpProposalHook {
    fn on_proposed(&self, _item: &MemoryItem) {}
}

pub struct MemoryApiImpl<R: RetrievalService> {
    retrieval: R,
    items: Mutex<HashMap<String, MemoryItem>>,
    next_id: Mutex<u64>,
    proposal_hook: Box<dyn MemoryProposalHook>,
}

impl<R: RetrievalService> MemoryApiImpl<R> {
    pub fn new(retrieval: R) -> Self {
        Self {
            retrieval,
            items: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
            proposal_hook: Box::new(NoOpProposalHook),
        }
    }

    /// Wire an SSE publisher or other listener for memory proposals.
    pub fn with_proposal_hook(mut self, hook: Box<dyn MemoryProposalHook>) -> Self {
        self.proposal_hook = hook;
        self
    }
}

#[async_trait]
impl<R: RetrievalService + 'static> MemoryEndpoints for MemoryApiImpl<R> {
    type Error = String;

    async fn list(
        &self,
        _project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<MemoryItem>, Self::Error> {
        let items = self.items.lock().unwrap();
        let limit = query.limit.unwrap_or(20).min(100);
        let offset = query.offset.unwrap_or(0);

        let mut results: Vec<MemoryItem> = items.values().cloned().collect();
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let total = results.len();
        let page: Vec<MemoryItem> = results.into_iter().skip(offset).take(limit).collect();

        Ok(ListResponse {
            items: page,
            has_more: total > offset + limit,
        })
    }

    async fn search(
        &self,
        project: &ProjectKey,
        query: &MemorySearchQuery,
    ) -> Result<Vec<MemoryItem>, Self::Error> {
        let response = self
            .retrieval
            .query(RetrievalQuery {
                project: project.clone(),
                query_text: query.q.clone(),
                mode: RetrievalMode::LexicalOnly,
                reranker: RerankerStrategy::None,
                limit: query.effective_limit(),
                metadata_filters: vec![],
                scoring_policy: None,
            })
            .await
            .map_err(|e| e.to_string())?;

        let items: Vec<MemoryItem> = response
            .results
            .into_iter()
            .map(|r| MemoryItem {
                id: r.chunk.chunk_id.to_string(),
                content: r.chunk.text,
                category: None,
                status: MemoryStatus::Accepted,
                source: None,
                confidence: Some(r.score),
                created_at: format!("{}", r.chunk.created_at),
            })
            .collect();

        Ok(items)
    }

    async fn create(
        &self,
        _project: &ProjectKey,
        request: &CreateMemoryRequest,
    ) -> Result<MemoryItem, Self::Error> {
        let mut next_id = self.next_id.lock().unwrap();
        let id = format!("mem_{}", *next_id);
        *next_id += 1;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let item = MemoryItem {
            id: id.clone(),
            content: request.content.clone(),
            category: request.category.clone(),
            status: MemoryStatus::Proposed,
            source: Some("assistant".to_owned()),
            confidence: None,
            created_at: format!("{now}"),
        };

        self.items.lock().unwrap().insert(id, item.clone());
        self.proposal_hook.on_proposed(&item);
        Ok(item)
    }

    async fn accept(&self, memory_id: &str) -> Result<(), Self::Error> {
        let mut items = self.items.lock().unwrap();
        if let Some(item) = items.get_mut(memory_id) {
            item.status = MemoryStatus::Accepted;
            Ok(())
        } else {
            Err(format!("memory not found: {memory_id}"))
        }
    }

    async fn reject(&self, memory_id: &str) -> Result<(), Self::Error> {
        let mut items = self.items.lock().unwrap();
        if let Some(item) = items.get_mut(memory_id) {
            item.status = MemoryStatus::Rejected;
            Ok(())
        } else {
            Err(format!("memory not found: {memory_id}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
    use crate::ingest::{IngestRequest, IngestService, SourceType};
    use crate::pipeline::{IngestPipeline, ParagraphChunker};
    use cairn_domain::{KnowledgeDocumentId, SourceId};
    use std::sync::Arc;

    #[tokio::test]
    async fn memory_search_delegates_to_retrieval() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let chunker = ParagraphChunker {
            max_chunk_size: 200,
        };
        let pipeline = IngestPipeline::new(store.clone(), chunker);

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_1"),
                source_id: SourceId::new("src_1"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Rust borrow checker ensures memory safety.".to_owned(),
            })
            .await
            .unwrap();

        let retrieval = InMemoryRetrieval::new(store);
        let api = MemoryApiImpl::new(retrieval);

        let results = api
            .search(
                &ProjectKey::new("t", "w", "p"),
                &MemorySearchQuery {
                    q: "borrow checker".to_owned(),
                    limit: Some(5),
                },
            )
            .await
            .unwrap();

        assert!(!results.is_empty());
        assert!(results[0].content.contains("borrow checker"));
    }

    #[tokio::test]
    async fn memory_crud_lifecycle() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let retrieval = InMemoryRetrieval::new(store);
        let api = MemoryApiImpl::new(retrieval);
        let project = ProjectKey::new("t", "w", "p");

        let item = api
            .create(
                &project,
                &CreateMemoryRequest {
                    content: "User prefers dark mode".to_owned(),
                    category: Some("preferences".to_owned()),
                },
            )
            .await
            .unwrap();

        assert_eq!(item.status, MemoryStatus::Proposed);

        api.accept(&item.id).await.unwrap();

        let list = api.list(&project, &ListQuery::default()).await.unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].status, MemoryStatus::Accepted);
    }
}
