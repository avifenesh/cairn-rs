//! Implementation of Worker 8's MemoryEndpoints backed by cairn-memory services.
//!
//! Wires the API-facing memory CRUD and search to the owned retrieval
//! pipeline, closing the seam between cairn-api and cairn-memory.

use async_trait::async_trait;
use chrono::{SecondsFormat, TimeZone, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_api::endpoints::ListQuery;
use cairn_api::http::ListResponse;
use cairn_api::memory_api::{
    CreateMemoryRequest, MemoryEndpoints, MemoryItem, MemorySearchQuery, MemoryStatus,
};
use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};

use crate::in_memory::InMemoryDocumentStore;
use crate::ingest::{ChunkRecord, SourceType};
use crate::pipeline::DocumentStore;
use crate::retrieval::{
    MetadataFilter, RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService,
};

/// Source ID marker for memory items stored in the document store.
const MEMORY_SOURCE_ID: &str = "__cairn_memory";

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
    store: Arc<InMemoryDocumentStore>,
    next_id: Mutex<u64>,
    proposal_hook: Box<dyn MemoryProposalHook>,
}

impl<R: RetrievalService> MemoryApiImpl<R> {
    pub fn new(retrieval: R, store: Arc<InMemoryDocumentStore>) -> Self {
        // Recover next_id from existing memory items so IDs never collide after restart.
        let max_id = store
            .all_chunks()
            .iter()
            .filter(|c| c.source_id.as_str() == MEMORY_SOURCE_ID)
            .filter_map(|c| {
                c.chunk_id
                    .as_str()
                    .strip_prefix("mem_")
                    .and_then(|n| n.parse::<u64>().ok())
            })
            .max()
            .unwrap_or(0);

        Self {
            retrieval,
            store,
            next_id: Mutex::new(max_id + 1),
            proposal_hook: Box::new(NoOpProposalHook),
        }
    }

    /// Wire an SSE publisher or other listener for memory proposals.
    pub fn with_proposal_hook(mut self, hook: Box<dyn MemoryProposalHook>) -> Self {
        self.proposal_hook = hook;
        self
    }
}

/// Reconstruct a `MemoryItem` from a `ChunkRecord` that was stored
/// with the `__cairn_memory` source marker.
fn chunk_to_memory_item(chunk: &ChunkRecord) -> MemoryItem {
    let meta = chunk.provenance_metadata.as_ref();

    let status = meta
        .and_then(|m| m.get("status"))
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "accepted" => MemoryStatus::Accepted,
            "rejected" => MemoryStatus::Rejected,
            _ => MemoryStatus::Proposed,
        })
        .unwrap_or(MemoryStatus::Proposed);

    let category = meta
        .and_then(|m| m.get("category"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned());

    let source = meta
        .and_then(|m| m.get("source"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned());

    let confidence = meta
        .and_then(|m| m.get("confidence"))
        .and_then(|v| v.as_f64());

    MemoryItem {
        id: chunk.chunk_id.as_str().to_owned(),
        content: chunk.text.clone(),
        category,
        status,
        source,
        confidence,
        created_at: format_memory_timestamp(chunk.created_at),
    }
}

fn format_memory_timestamp(created_at_ms: u64) -> String {
    i64::try_from(created_at_ms)
        .ok()
        .and_then(|millis| Utc.timestamp_millis_opt(millis).single())
        .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_else(|| created_at_ms.to_string())
}

#[async_trait]
impl<R: RetrievalService + 'static> MemoryEndpoints for MemoryApiImpl<R> {
    type Error = String;

    async fn list(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<MemoryItem>, Self::Error> {
        let chunks = self.store.all_chunks();
        let limit = query.limit.unwrap_or(20).min(100);
        let offset = query.offset.unwrap_or(0);

        let mut memory_chunks: Vec<&ChunkRecord> = chunks
            .iter()
            .filter(|c| c.source_id.as_str() == MEMORY_SOURCE_ID && &c.project == project)
            .collect();
        memory_chunks.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.chunk_id.as_str().cmp(a.chunk_id.as_str()))
        });

        let mut results: Vec<MemoryItem> = memory_chunks
            .into_iter()
            .map(chunk_to_memory_item)
            .collect();

        if let Some(status) = query.status.as_deref() {
            let expected = match status {
                "proposed" => MemoryStatus::Proposed,
                "accepted" => MemoryStatus::Accepted,
                "rejected" => MemoryStatus::Rejected,
                other => return Err(format!("invalid memory status: {other}")),
            };
            results.retain(|item| item.status == expected);
        }

        if let Some(category) = query.category.as_deref() {
            results.retain(|item| item.category.as_deref() == Some(category));
        }

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
                metadata_filters: vec![MetadataFilter {
                    key: "type".to_owned(),
                    value: "memory_item".to_owned(),
                }],
                scoring_policy: None,
            })
            .await
            .map_err(|e| e.to_string())?;

        let items: Vec<MemoryItem> = response
            .results
            .into_iter()
            .filter(|r| r.chunk.source_id.as_str() == MEMORY_SOURCE_ID)
            .map(|r| {
                let mut item = chunk_to_memory_item(&r.chunk);
                item.confidence = Some(r.score);
                item
            })
            .collect();

        Ok(items)
    }

    async fn create(
        &self,
        project: &ProjectKey,
        request: &CreateMemoryRequest,
    ) -> Result<MemoryItem, Self::Error> {
        let id = {
            let mut next_id = self.next_id.lock().unwrap();
            let id = format!("mem_{}", *next_id);
            *next_id += 1;
            id
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let metadata = serde_json::json!({
            "type": "memory_item",
            "status": "proposed",
            "category": request.category,
            "source": "assistant",
        });

        let chunk = ChunkRecord {
            chunk_id: ChunkId::new(&id),
            document_id: KnowledgeDocumentId::new(&id),
            source_id: SourceId::new(MEMORY_SOURCE_ID),
            source_type: SourceType::PlainText,
            project: project.clone(),
            text: request.content.clone(),
            position: 0,
            created_at: now,
            updated_at: None,
            provenance_metadata: Some(metadata),
            credibility_score: None,
            graph_linkage: None,
            embedding: None,
            content_hash: None,
            entities: vec![],
            embedding_model_id: None,
            needs_reembed: false,
        };

        self.store
            .insert_chunks(&[chunk])
            .await
            .map_err(|e| e.to_string())?;

        let item = MemoryItem {
            id: id.clone(),
            content: request.content.clone(),
            category: request.category.clone(),
            status: MemoryStatus::Proposed,
            source: Some("assistant".to_owned()),
            confidence: None,
            created_at: format_memory_timestamp(now),
        };

        self.proposal_hook.on_proposed(&item);
        Ok(item)
    }

    async fn accept(&self, project: &ProjectKey, memory_id: &str) -> Result<(), Self::Error> {
        let mut chunks = self.store.chunks_mut();
        let chunk = chunks
            .iter_mut()
            .find(|c| {
                c.chunk_id.as_str() == memory_id
                    && c.source_id.as_str() == MEMORY_SOURCE_ID
                    && &c.project == project
            })
            .ok_or_else(|| format!("memory not found: {memory_id}"))?;

        if let Some(obj) = chunk
            .provenance_metadata
            .as_mut()
            .and_then(|m| m.as_object_mut())
        {
            obj.insert("status".to_owned(), serde_json::json!("accepted"));
        }
        Ok(())
    }

    async fn reject(&self, project: &ProjectKey, memory_id: &str) -> Result<(), Self::Error> {
        let mut chunks = self.store.chunks_mut();
        let chunk = chunks
            .iter_mut()
            .find(|c| {
                c.chunk_id.as_str() == memory_id
                    && c.source_id.as_str() == MEMORY_SOURCE_ID
                    && &c.project == project
            })
            .ok_or_else(|| format!("memory not found: {memory_id}"))?;

        if let Some(obj) = chunk
            .provenance_metadata
            .as_mut()
            .and_then(|m| m.as_object_mut())
        {
            obj.insert("status".to_owned(), serde_json::json!("rejected"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Corpus management implementation (RFC 003)
// ---------------------------------------------------------------------------

use cairn_api::memory_api::{
    AddDocumentToCorpusRequest, AddSourceTagsRequest, CorpusEndpoints, CorpusRecord,
    CreateCorpusRequest, SourceTagsEndpoints, SourceTagsResponse,
};

/// Corpus API implementation backed by InMemoryDocumentStore.
pub struct CorpusApiImpl {
    store: std::sync::Arc<InMemoryDocumentStore>,
    corpora: Mutex<Vec<CorpusEntry>>,
}

/// Internal corpus entry with project scope.
struct CorpusEntry {
    corpus_id: String,
    name: String,
    description: Option<String>,
    project: ProjectKey,
    document_ids: Vec<String>,
}

impl CorpusApiImpl {
    pub fn new(store: std::sync::Arc<InMemoryDocumentStore>) -> Self {
        Self {
            store,
            corpora: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl CorpusEndpoints for CorpusApiImpl {
    type Error = String;

    async fn create_corpus(
        &self,
        project: &ProjectKey,
        request: &CreateCorpusRequest,
    ) -> Result<CorpusRecord, Self::Error> {
        let mut corpora = self.corpora.lock().unwrap();
        let id = format!("corpus_{}", corpora.len() + 1);
        corpora.push(CorpusEntry {
            corpus_id: id.clone(),
            name: request.name.clone(),
            description: request.description.clone(),
            project: project.clone(),
            document_ids: Vec::new(),
        });
        Ok(CorpusRecord {
            corpus_id: id,
            name: request.name.clone(),
            description: request.description.clone(),
            document_count: 0,
        })
    }

    async fn get_corpus(&self, corpus_id: &str) -> Result<Option<CorpusRecord>, Self::Error> {
        let corpora = self.corpora.lock().unwrap();
        Ok(corpora.iter().find(|c| c.corpus_id == corpus_id).map(|c| {
            // Count documents: both directly added and those ingested with this corpus_id.
            let chunks = self.store.all_current_chunks();
            let ingested_doc_ids: std::collections::HashSet<String> = chunks
                .iter()
                .filter(|ch| {
                    ch.provenance_metadata
                        .as_ref()
                        .and_then(|m| m.get("corpus_id"))
                        .and_then(|v| v.as_str())
                        == Some(corpus_id)
                })
                .map(|ch| ch.document_id.as_str().to_owned())
                .collect();

            let mut all_doc_ids: std::collections::HashSet<String> = ingested_doc_ids;
            for did in &c.document_ids {
                all_doc_ids.insert(did.clone());
            }

            CorpusRecord {
                corpus_id: c.corpus_id.clone(),
                name: c.name.clone(),
                description: c.description.clone(),
                document_count: all_doc_ids.len() as u32,
            }
        }))
    }

    async fn list_corpora(&self, project: &ProjectKey) -> Result<Vec<CorpusRecord>, Self::Error> {
        let corpora = self.corpora.lock().unwrap();
        Ok(corpora
            .iter()
            .filter(|c| &c.project == project)
            .map(|c| CorpusRecord {
                corpus_id: c.corpus_id.clone(),
                name: c.name.clone(),
                description: c.description.clone(),
                document_count: c.document_ids.len() as u32,
            })
            .collect())
    }

    async fn add_document_to_corpus(
        &self,
        corpus_id: &str,
        request: &AddDocumentToCorpusRequest,
    ) -> Result<(), Self::Error> {
        let mut corpora = self.corpora.lock().unwrap();
        let corpus = corpora
            .iter_mut()
            .find(|c| c.corpus_id == corpus_id)
            .ok_or_else(|| format!("corpus not found: {corpus_id}"))?;

        if !corpus.document_ids.contains(&request.document_id) {
            corpus.document_ids.push(request.document_id.clone());
        }

        // Tag existing chunks from that document with corpus_id in provenance_metadata.
        drop(corpora);
        let mut store_chunks = self.store.chunks_mut();
        for chunk in store_chunks.iter_mut() {
            if chunk.document_id.as_str() == request.document_id {
                let mut meta = chunk
                    .provenance_metadata
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({}));
                meta.as_object_mut()
                    .unwrap()
                    .insert("corpus_id".to_owned(), serde_json::json!(corpus_id));
                chunk.provenance_metadata = Some(meta);
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Source tags implementation (RFC 003)
// ---------------------------------------------------------------------------

/// Source tags API implementation backed by InMemoryDocumentStore.
pub struct SourceTagsApiImpl {
    store: std::sync::Arc<InMemoryDocumentStore>,
    tags: Mutex<HashMap<String, Vec<String>>>,
}

impl SourceTagsApiImpl {
    pub fn new(store: std::sync::Arc<InMemoryDocumentStore>) -> Self {
        Self {
            store,
            tags: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl SourceTagsEndpoints for SourceTagsApiImpl {
    type Error = String;

    async fn get_source_tags(&self, source_id: &str) -> Result<SourceTagsResponse, Self::Error> {
        let tags = self.tags.lock().unwrap();
        let source_tags = tags.get(source_id).cloned().unwrap_or_default();
        Ok(SourceTagsResponse {
            source_id: source_id.to_owned(),
            tags: source_tags,
        })
    }

    async fn add_source_tags(
        &self,
        source_id: &str,
        request: &AddSourceTagsRequest,
    ) -> Result<SourceTagsResponse, Self::Error> {
        let mut tags = self.tags.lock().unwrap();
        let entry = tags.entry(source_id.to_owned()).or_default();
        for tag in &request.tags {
            if !entry.contains(tag) {
                entry.push(tag.clone());
            }
        }
        let result = entry.clone();
        drop(tags);

        // Retroactively tag chunks from this source in provenance_metadata.
        let mut store_chunks = self.store.chunks_mut();
        for chunk in store_chunks.iter_mut() {
            if chunk.source_id.as_str() == source_id {
                let mut meta = chunk
                    .provenance_metadata
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({}));
                let obj = meta.as_object_mut().unwrap();
                let existing: Vec<String> = obj
                    .get("tags")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let mut merged = existing;
                for tag in &request.tags {
                    if !merged.contains(tag) {
                        merged.push(tag.clone());
                    }
                }
                obj.insert("tags".to_owned(), serde_json::json!(merged));
                chunk.provenance_metadata = Some(meta);
            }
        }

        Ok(SourceTagsResponse {
            source_id: source_id.to_owned(),
            tags: result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
    use crate::ingest::{IngestRequest, IngestService, SourceType};
    use crate::pipeline::{IngestPipeline, ParagraphChunker};
    use std::sync::Arc;

    #[tokio::test]
    async fn memory_search_delegates_to_retrieval() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let project = ProjectKey::new("t", "w", "p");
        let retrieval = InMemoryRetrieval::new(store.clone());
        let api = MemoryApiImpl::new(retrieval, store);

        api.create(
            &project,
            &CreateMemoryRequest {
                content: "Rust borrow checker ensures memory safety.".to_owned(),
                category: Some("facts".to_owned()),
            },
        )
        .await
        .unwrap();

        let results = api
            .search(
                &project,
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
        let retrieval = InMemoryRetrieval::new(store.clone());
        let api = MemoryApiImpl::new(retrieval, store);
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

        api.accept(&project, &item.id).await.unwrap();

        let list = api.list(&project, &ListQuery::default()).await.unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].status, MemoryStatus::Accepted);
    }

    #[tokio::test]
    async fn memory_list_filters_category_before_paginating() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let retrieval = InMemoryRetrieval::new(store.clone());
        let api = MemoryApiImpl::new(retrieval, store);
        let project = ProjectKey::new("t", "w", "p");

        api.create(
            &project,
            &CreateMemoryRequest {
                content: "Project memory one".to_owned(),
                category: Some("project".to_owned()),
            },
        )
        .await
        .unwrap();
        api.create(
            &project,
            &CreateMemoryRequest {
                content: "Ops memory".to_owned(),
                category: Some("ops".to_owned()),
            },
        )
        .await
        .unwrap();
        api.create(
            &project,
            &CreateMemoryRequest {
                content: "Project memory two".to_owned(),
                category: Some("project".to_owned()),
            },
        )
        .await
        .unwrap();

        let list = api
            .list(
                &project,
                &ListQuery {
                    limit: Some(2),
                    offset: Some(0),
                    status: None,
                    category: Some("project".to_owned()),
                },
            )
            .await
            .unwrap();
        assert_eq!(list.items.len(), 2);
        assert!(list
            .items
            .iter()
            .all(|item| item.category.as_deref() == Some("project")));
        assert!(!list.has_more);
    }

    #[tokio::test]
    async fn memory_list_and_mutations_enforce_project_scope() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let retrieval = InMemoryRetrieval::new(store.clone());
        let api = MemoryApiImpl::new(retrieval, store);
        let project_a = ProjectKey::new("t", "w", "project-a");
        let project_b = ProjectKey::new("t", "w", "project-b");

        let item_a = api
            .create(
                &project_a,
                &CreateMemoryRequest {
                    content: "Project A memory".to_owned(),
                    category: Some("project".to_owned()),
                },
            )
            .await
            .unwrap();

        api.create(
            &project_b,
            &CreateMemoryRequest {
                content: "Project B memory".to_owned(),
                category: Some("project".to_owned()),
            },
        )
        .await
        .unwrap();

        let list_a = api.list(&project_a, &ListQuery::default()).await.unwrap();
        assert_eq!(list_a.items.len(), 1);
        assert_eq!(list_a.items[0].id, item_a.id);

        let accept_err = api.accept(&project_b, &item_a.id).await.unwrap_err();
        assert!(accept_err.starts_with("memory not found:"));

        let reject_err = api.reject(&project_b, &item_a.id).await.unwrap_err();
        assert!(reject_err.starts_with("memory not found:"));

        api.accept(&project_a, &item_a.id).await.unwrap();
        let accepted = api
            .list(
                &project_a,
                &ListQuery {
                    status: Some("accepted".to_owned()),
                    ..ListQuery::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(accepted.items.len(), 1);
        assert_eq!(accepted.items[0].id, item_a.id);
    }

    #[tokio::test]
    async fn memory_search_applies_memory_filter_before_limit() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let retrieval = InMemoryRetrieval::new(store.clone());
        let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
        let api = MemoryApiImpl::new(retrieval, store);
        let project = ProjectKey::new("t", "w", "p");

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_1"),
                source_id: SourceId::new("src_regular"),
                source_type: SourceType::PlainText,
                project: project.clone(),
                content: "Borrow checker guides regular documentation.".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let created = api
            .create(
                &project,
                &CreateMemoryRequest {
                    content: "Borrow checker is a memory worth keeping.".to_owned(),
                    category: Some("facts".to_owned()),
                },
            )
            .await
            .unwrap();

        let results = api
            .search(
                &project,
                &MemorySearchQuery {
                    q: "borrow checker".to_owned(),
                    limit: Some(1),
                },
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, created.id);
    }

    #[tokio::test]
    async fn memory_list_sorts_by_raw_timestamp_not_truncated_string() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let retrieval = InMemoryRetrieval::new(store.clone());
        let api = MemoryApiImpl::new(retrieval, store.clone());
        let project = ProjectKey::new("t", "w", "p");

        store
            .insert_chunks(&[
                ChunkRecord {
                    chunk_id: ChunkId::new("mem_1"),
                    document_id: KnowledgeDocumentId::new("mem_1"),
                    source_id: SourceId::new(MEMORY_SOURCE_ID),
                    source_type: SourceType::PlainText,
                    project: project.clone(),
                    text: "Older same-second memory".to_owned(),
                    position: 0,
                    created_at: 1_700_000_000_123,
                    updated_at: None,
                    provenance_metadata: Some(serde_json::json!({
                        "type": "memory_item",
                        "status": "proposed",
                        "category": "project",
                        "source": "assistant",
                    })),
                    credibility_score: None,
                    graph_linkage: None,
                    embedding: None,
                    content_hash: None,
                    entities: vec![],
                    embedding_model_id: None,
                    needs_reembed: false,
                },
                ChunkRecord {
                    chunk_id: ChunkId::new("mem_2"),
                    document_id: KnowledgeDocumentId::new("mem_2"),
                    source_id: SourceId::new(MEMORY_SOURCE_ID),
                    source_type: SourceType::PlainText,
                    project: project.clone(),
                    text: "Newer same-second memory".to_owned(),
                    position: 0,
                    created_at: 1_700_000_000_456,
                    updated_at: None,
                    provenance_metadata: Some(serde_json::json!({
                        "type": "memory_item",
                        "status": "proposed",
                        "category": "project",
                        "source": "assistant",
                    })),
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

        let list = api.list(&project, &ListQuery::default()).await.unwrap();
        assert_eq!(list.items.len(), 2);
        assert_eq!(list.items[0].id, "mem_2");
        assert_eq!(list.items[1].id, "mem_1");
    }
}
