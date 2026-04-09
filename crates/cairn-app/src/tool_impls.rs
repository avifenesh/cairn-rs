//! Concrete built-in tool implementations for cairn-app.
//!
//! cairn-tools cannot depend on cairn-memory (circular dep: cairn-api →
//! cairn-tools → cairn-memory → cairn-api).  cairn-app depends on both, so
//! it is the right place to bridge the two crates with real implementations.
//!
//! # Provided implementations
//!
//! | Type                    | Backed by                                         |
//! |-------------------------|---------------------------------------------------|
//! | `ConcreteMemorySearchTool` | `Arc<dyn RetrievalService>` from cairn-memory  |
//! | `ConcreteMemoryStoreTool`  | `Arc<dyn IngestService>` from cairn-memory     |
//!
//! # Wiring
//!
//! Call [`build_tool_registry`] with the live services, then attach the
//! resulting `BuiltinToolRegistry` to the `RuntimeExecutePhase` builder.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::{
    ingest::{IngestRequest, IngestService, SourceType},
    retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService},
};
use cairn_tools::builtins::{BuiltinToolRegistry, ToolError, ToolHandler, ToolResult, ToolTier};
use serde_json::Value;

// ── ConcreteMemorySearchTool ──────────────────────────────────────────────────

/// Real `memory_search` — calls [`RetrievalService::query`] with the LLM's args.
pub struct ConcreteMemorySearchTool {
    retrieval: Arc<dyn RetrievalService>,
}

impl ConcreteMemorySearchTool {
    pub fn new(retrieval: Arc<dyn RetrievalService>) -> Self {
        Self { retrieval }
    }
}

#[async_trait]
impl ToolHandler for ConcreteMemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Core
    }

    fn description(&self) -> &str {
        "Search the agent's memory for relevant information. \
         Returns the most relevant text chunks from previously stored knowledge. \
         Use this before answering questions that may require prior context."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default 5, max 20)",
                    "default": 5,
                    "minimum": 1,
                    "maximum": 20
                },
                "mode": {
                    "type": "string",
                    "description": "Retrieval mode: lexical (keyword match), vector (semantic), or hybrid",
                    "enum": ["lexical", "vector", "hybrid"],
                    "default": "lexical"
                }
            }
        })
    }

    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let query_text = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "query".into(),
                message: "required string".into(),
            })?
            .to_owned();

        if query_text.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "query".into(),
                message: "must not be empty".into(),
            });
        }

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(20))
            .unwrap_or(5);

        let mode = match args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("lexical")
        {
            "vector" => RetrievalMode::VectorOnly,
            "hybrid" => RetrievalMode::Hybrid,
            _ => RetrievalMode::LexicalOnly,
        };

        let query = RetrievalQuery {
            project: project.clone(),
            query_text,
            mode,
            reranker: RerankerStrategy::None,
            limit,
            metadata_filters: vec![],
            scoring_policy: None,
        };

        match self.retrieval.query(query).await {
            Ok(resp) => {
                let results: Vec<Value> = resp
                    .results
                    .into_iter()
                    .map(|r| {
                        serde_json::json!({
                            "chunk_id":    r.chunk.chunk_id.as_str(),
                            "text":        r.chunk.text,
                            "score":       r.score,
                            "source_id":   r.chunk.source_id.as_str(),
                            "document_id": r.chunk.document_id.as_str(),
                        })
                    })
                    .collect();
                let total = results.len();
                Ok(ToolResult::ok(serde_json::json!({
                    "results": results,
                    "total":   total,
                })))
            }
            Err(e) => Err(ToolError::Transient(format!("retrieval failed: {e}"))),
        }
    }
}

// ── ConcreteMemoryStoreTool ───────────────────────────────────────────────────

/// Real `memory_store` — calls [`IngestService::submit`] to ingest new content.
pub struct ConcreteMemoryStoreTool {
    ingest: Arc<dyn IngestService>,
}

impl ConcreteMemoryStoreTool {
    pub fn new(ingest: Arc<dyn IngestService>) -> Self {
        Self { ingest }
    }
}

#[async_trait]
impl ToolHandler for ConcreteMemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Core
    }

    fn description(&self) -> &str {
        "Store new knowledge into the agent's memory for future retrieval. \
         Use this to remember summaries, decisions, or facts discovered during execution. \
         The stored content becomes searchable via memory_search."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["content"],
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The text to store in memory"
                },
                "source_id": {
                    "type": "string",
                    "description": "Source label for the content (default: 'agent')",
                    "default": "agent"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional tags for later filtering"
                }
            }
        })
    }

    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "content".into(),
                message: "required string".into(),
            })?;

        if content.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "content".into(),
                message: "must not be empty".into(),
            });
        }

        let source_label = args
            .get("source_id")
            .and_then(|v| v.as_str())
            .unwrap_or("agent")
            .to_owned();

        let tags: Vec<String> = args
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        // Build a unique document ID: timestamp_ms + FNV-1a hash of content.
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let content_hash: u32 = content.as_bytes().iter().fold(0x811c9dc5u32, |h, &b| {
            h.wrapping_mul(0x01000193) ^ (b as u32)
        });
        let document_id = KnowledgeDocumentId::new(format!("mem_{ts_ms}_{content_hash:08x}"));
        let source_id = SourceId::new(&source_label);

        self.ingest
            .submit(IngestRequest {
                document_id: document_id.clone(),
                source_id: source_id.clone(),
                source_type: SourceType::PlainText,
                project: project.clone(),
                content: content.to_owned(),
                tags,
                corpus_id: None,
                import_id: None,
                bundle_source_id: None,
            })
            .await
            .map_err(|e| ToolError::Transient(format!("ingest failed: {e}")))?;

        Ok(ToolResult::ok(serde_json::json!({
            "document_id": document_id.as_str(),
            "source_id":   source_id.as_str(),
            "stored":      true,
        })))
    }
}

// ── Registry builder ──────────────────────────────────────────────────────────

/// Build a [`BuiltinToolRegistry`] pre-populated with the concrete memory tools.
///
/// Call this at startup and attach the result to `RuntimeExecutePhase::builder()
/// .tool_registry(Arc::new(registry))`.
pub fn build_tool_registry(
    retrieval: Arc<dyn RetrievalService>,
    ingest: Arc<dyn IngestService>,
) -> BuiltinToolRegistry {
    BuiltinToolRegistry::new()
        .register(Arc::new(ConcreteMemorySearchTool::new(retrieval)))
        .register(Arc::new(ConcreteMemoryStoreTool::new(ingest)))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ProjectKey;
    use cairn_memory::{
        in_memory::{InMemoryDocumentStore, InMemoryRetrieval},
        ingest::{IngestRequest, SourceType},
        pipeline::{IngestPipeline, ParagraphChunker},
        IngestService,
    };
    use std::sync::Arc;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    fn make_ingest() -> (
        Arc<InMemoryDocumentStore>,
        Arc<IngestPipeline<Arc<InMemoryDocumentStore>, ParagraphChunker>>,
    ) {
        let store = Arc::new(InMemoryDocumentStore::new());
        let pipeline = Arc::new(IngestPipeline::new(
            store.clone(),
            ParagraphChunker::default(),
        ));
        (store, pipeline)
    }

    // ── memory_search ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn search_finds_ingested_content() {
        let (store, pipeline) = make_ingest();
        pipeline
            .submit(IngestRequest {
                document_id: cairn_domain::KnowledgeDocumentId::new("doc_1"),
                source_id: cairn_domain::SourceId::new("test"),
                source_type: SourceType::PlainText,
                project: project(),
                content: "cairn-rs is an event-sourced AI agent runtime in Rust.".to_owned(),
                tags: vec![],
                corpus_id: None,
                import_id: None,
                bundle_source_id: None,
            })
            .await
            .unwrap();

        let tool = ConcreteMemorySearchTool::new(Arc::new(InMemoryRetrieval::new(store)));
        let result = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "Rust event sourced runtime"
                }),
            )
            .await
            .unwrap();

        let total = result.output["total"].as_u64().unwrap();
        assert!(total > 0, "should find at least one chunk");
        let text = result.output["results"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("cairn") || text.contains("Rust"),
            "result must contain relevant content"
        );
    }

    #[tokio::test]
    async fn search_returns_empty_on_no_match() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let tool = ConcreteMemorySearchTool::new(Arc::new(InMemoryRetrieval::new(store)));
        let result = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "completely unrelated xyz123"
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["total"], 0);
    }

    #[tokio::test]
    async fn search_rejects_empty_query() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let tool = ConcreteMemorySearchTool::new(Arc::new(InMemoryRetrieval::new(store)));
        let err = tool
            .execute(&project(), serde_json::json!({ "query": "" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn search_respects_limit() {
        let (store, pipeline) = make_ingest();
        for i in 0..5 {
            pipeline
                .submit(IngestRequest {
                    document_id: cairn_domain::KnowledgeDocumentId::new(format!("doc_{i}")),
                    source_id: cairn_domain::SourceId::new("test"),
                    source_type: SourceType::PlainText,
                    project: project(),
                    content: format!("Document {i}: information about cairn sessions and runs."),
                    tags: vec![],
                    corpus_id: None,
                    import_id: None,
                    bundle_source_id: None,
                })
                .await
                .unwrap();
        }
        let tool = ConcreteMemorySearchTool::new(Arc::new(InMemoryRetrieval::new(store)));
        let result = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "cairn sessions",
                    "limit": 2
                }),
            )
            .await
            .unwrap();
        let returned = result.output["results"].as_array().unwrap().len();
        assert!(returned <= 2, "limit=2 must not return more than 2 chunks");
    }

    // ── memory_store ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn store_returns_document_id() {
        let (_, pipeline) = make_ingest();
        let tool = ConcreteMemoryStoreTool::new(pipeline);
        let result = tool
            .execute(
                &project(),
                serde_json::json!({
                    "content": "The sky is blue because of Rayleigh scattering."
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["stored"], true);
        assert!(result.output["document_id"]
            .as_str()
            .unwrap()
            .starts_with("mem_"));
    }

    #[tokio::test]
    async fn stored_content_is_searchable() {
        let (store, pipeline) = make_ingest();
        let store_tool = ConcreteMemoryStoreTool::new(pipeline);
        let search_tool = ConcreteMemorySearchTool::new(Arc::new(InMemoryRetrieval::new(store)));

        store_tool
            .execute(
                &project(),
                serde_json::json!({
                    "content": "cairn-rs uses lexical search for memory retrieval"
                }),
            )
            .await
            .unwrap();

        let result = search_tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "cairn memory retrieval"
                }),
            )
            .await
            .unwrap();

        let total = result.output["total"].as_u64().unwrap();
        assert!(
            total > 0,
            "just-stored content must be immediately searchable"
        );
    }

    #[tokio::test]
    async fn store_rejects_empty_content() {
        let (_, pipeline) = make_ingest();
        let tool = ConcreteMemoryStoreTool::new(pipeline);
        let err = tool
            .execute(&project(), serde_json::json!({ "content": "  " }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn store_preserves_custom_source_id() {
        let (_, pipeline) = make_ingest();
        let tool = ConcreteMemoryStoreTool::new(pipeline);
        let result = tool
            .execute(
                &project(),
                serde_json::json!({
                    "content":   "Research finding: neural networks require data.",
                    "source_id": "research_agent"
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["source_id"], "research_agent");
    }

    // ── build_tool_registry ───────────────────────────────────────────────────

    #[tokio::test]
    async fn registry_dispatches_memory_search() {
        let (store, pipeline) = make_ingest();
        pipeline
            .submit(IngestRequest {
                document_id: cairn_domain::KnowledgeDocumentId::new("doc_reg"),
                source_id: cairn_domain::SourceId::new("test"),
                source_type: SourceType::PlainText,
                project: project(),
                content: "cairn-rs event sourcing and approval gates".to_owned(),
                tags: vec![],
                corpus_id: None,
                import_id: None,
                bundle_source_id: None,
            })
            .await
            .unwrap();

        let retrieval = Arc::new(InMemoryRetrieval::new(store)) as Arc<dyn RetrievalService>;
        let ingest = pipeline as Arc<dyn IngestService>;
        let registry = build_tool_registry(retrieval, ingest);

        let result = registry
            .execute(
                "memory_search",
                &project(),
                serde_json::json!({ "query": "event sourcing" }),
            )
            .await
            .unwrap();
        assert!(result.output["total"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn registry_dispatches_memory_store() {
        let (_, pipeline) = make_ingest();
        let retrieval = Arc::new(InMemoryRetrieval::new(Arc::new(
            InMemoryDocumentStore::new(),
        ))) as Arc<dyn RetrievalService>;
        let ingest = pipeline as Arc<dyn IngestService>;
        let registry = build_tool_registry(retrieval, ingest);

        let result = registry
            .execute(
                "memory_store",
                &project(),
                serde_json::json!({ "content": "test fact" }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["stored"], true);
    }

    #[tokio::test]
    async fn registry_returns_error_for_unknown_tool() {
        let (_, pipeline) = make_ingest();
        let retrieval = Arc::new(InMemoryRetrieval::new(Arc::new(
            InMemoryDocumentStore::new(),
        ))) as Arc<dyn RetrievalService>;
        let registry = build_tool_registry(retrieval, pipeline as Arc<dyn IngestService>);

        let err = registry
            .execute("nonexistent_tool", &project(), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown tool"));
    }
}
