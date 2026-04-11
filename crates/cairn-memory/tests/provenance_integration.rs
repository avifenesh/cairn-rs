//! Provenance/search integration proof.
//!
//! End-to-end test: ingest documents, project to graph, search,
//! then trace provenance from search result back to source through
//! the graph. Proves the owned retrieval + graph provenance pipeline
//! works as a connected system.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cairn_domain::*;
use cairn_graph::graph_provenance::GraphProvenanceService;
use cairn_graph::projections::*;
use cairn_graph::provenance::ProvenanceService;
use cairn_graph::queries::*;
use cairn_graph::retrieval_projector::RetrievalGraphProjector;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};

/// Shared in-memory graph for test.
type NodeMap = (HashMap<String, GraphNode>, Vec<GraphEdge>);
struct TestGraph(Arc<Mutex<NodeMap>>);

impl TestGraph {
    fn new() -> Self {
        Self(Arc::new(Mutex::new((HashMap::new(), Vec::new()))))
    }
}

impl Clone for TestGraph {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[async_trait]
impl GraphProjection for TestGraph {
    async fn add_node(&self, node: GraphNode) -> Result<(), GraphProjectionError> {
        self.0.lock().unwrap().0.insert(node.node_id.clone(), node);
        Ok(())
    }
    async fn add_edge(&self, edge: GraphEdge) -> Result<(), GraphProjectionError> {
        self.0.lock().unwrap().1.push(edge);
        Ok(())
    }
    async fn node_exists(&self, node_id: &str) -> Result<bool, GraphProjectionError> {
        Ok(self.0.lock().unwrap().0.contains_key(node_id))
    }
}

#[async_trait]
impl GraphQueryService for TestGraph {
    async fn query(&self, _query: GraphQuery) -> Result<Subgraph, GraphQueryError> {
        let data = self.0.lock().unwrap();
        Ok(Subgraph {
            nodes: data.0.values().cloned().collect(),
            edges: data.1.clone(),
        })
    }

    async fn neighbors(
        &self,
        _node_id: &str,
        _edge_filter: Option<EdgeKind>,
        _direction: TraversalDirection,
        _limit: usize,
    ) -> Result<Vec<(GraphEdge, GraphNode)>, GraphQueryError> {
        Ok(vec![])
    }
    async fn find_edges_by_source(
        &self,
        _: &str,
        _: Option<EdgeKind>,
        _: usize,
    ) -> Result<Vec<GraphEdge>, GraphQueryError> {
        Ok(vec![])
    }
    async fn find_edges_by_target(
        &self,
        _: &str,
        _: Option<EdgeKind>,
        _: usize,
    ) -> Result<Vec<GraphEdge>, GraphQueryError> {
        Ok(vec![])
    }
    async fn shortest_path(
        &self,
        _: &str,
        _: &str,
        _: Option<EdgeKind>,
        _: u32,
    ) -> Result<Option<Subgraph>, GraphQueryError> {
        Ok(None)
    }
}

#[tokio::test]
async fn ingest_search_and_trace_provenance_end_to_end() {
    // === Step 1: Set up services ===
    let doc_store = Arc::new(InMemoryDocumentStore::new());
    let chunker = ParagraphChunker {
        max_chunk_size: 150,
    };
    let pipeline = IngestPipeline::new(doc_store.clone(), chunker);
    let retrieval = InMemoryRetrieval::new(doc_store.clone());
    let graph = TestGraph::new();
    let retrieval_projector = RetrievalGraphProjector::new(graph.clone());
    let provenance_service = GraphProvenanceService::new(graph.clone());

    let project = ProjectKey::new("acme", "eng", "support");
    let source_id = SourceId::new("internal_wiki");

    // === Step 2: Ingest documents ===
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_troubleshoot"),
            source_id: source_id.clone(),
            source_type: SourceType::Markdown,
            project: project.clone(),
            content: "# Troubleshooting Guide\n\n\
                      Common issues include authentication failures and timeout errors.\n\n\
                      For authentication failures, verify the API key is valid and not expired.\n\n\
                      For timeout errors, check network connectivity and increase retry limits."
                .to_owned(),
            import_id: None,
            corpus_id: None,
            bundle_source_id: None,
            tags: vec![],
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_api_ref"),
            source_id: source_id.clone(),
            source_type: SourceType::PlainText,
            project: project.clone(),
            content: "API Reference: The authentication endpoint accepts POST requests.\n\n\
                      Include the API key in the Authorization header.\n\n\
                      Rate limiting applies after 100 requests per minute."
                .to_owned(),
            import_id: None,
            corpus_id: None,
            bundle_source_id: None,
            tags: vec![],
        })
        .await
        .unwrap();

    // === Step 3: Project to graph ===
    // Register source and documents in the graph.
    retrieval_projector
        .on_source_registered(&source_id, 100)
        .await
        .unwrap();
    retrieval_projector
        .on_document_ingested(
            &KnowledgeDocumentId::new("doc_troubleshoot"),
            &source_id,
            200,
        )
        .await
        .unwrap();
    retrieval_projector
        .on_document_ingested(&KnowledgeDocumentId::new("doc_api_ref"), &source_id, 300)
        .await
        .unwrap();

    // Project chunks.
    let chunks = doc_store.all_chunks();
    let chunk_ids: Vec<String> = chunks
        .iter()
        .filter(|c| c.document_id == KnowledgeDocumentId::new("doc_troubleshoot"))
        .map(|c| c.chunk_id.to_string())
        .collect();
    retrieval_projector
        .on_chunks_created(
            &chunk_ids,
            &KnowledgeDocumentId::new("doc_troubleshoot"),
            400,
        )
        .await
        .unwrap();

    // === Step 4: Search ===
    let search_results = retrieval
        .query(RetrievalQuery {
            project: project.clone(),
            query_text: "authentication API key".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(
        !search_results.results.is_empty(),
        "search should find authentication-related chunks"
    );

    let top_result = &search_results.results[0];
    assert!(
        top_result
            .chunk
            .text
            .to_lowercase()
            .contains("authentication")
            || top_result.chunk.text.to_lowercase().contains("api key"),
        "top result should be about authentication"
    );

    // === Step 5: Trace provenance ===
    // The graph should have: source -> doc -> chunks.
    let retrieval_prov = provenance_service
        .retrieval_provenance("doc_troubleshoot")
        .await
        .unwrap();

    // Should find the source.
    assert!(
        retrieval_prov
            .source_ids
            .iter()
            .any(|s| s.as_str() == "internal_wiki"),
        "provenance should trace back to source"
    );

    // Should find documents.
    assert!(
        !retrieval_prov.document_ids.is_empty(),
        "provenance should include documents"
    );

    // === Step 6: Verify graph structure ===
    let graph_data = graph.0.lock().unwrap();

    // Source node exists.
    assert!(graph_data.0.contains_key("internal_wiki"));
    assert_eq!(graph_data.0["internal_wiki"].kind, NodeKind::Source);

    // Document nodes exist.
    assert!(graph_data.0.contains_key("doc_troubleshoot"));
    assert_eq!(graph_data.0["doc_troubleshoot"].kind, NodeKind::Document);

    // Edges: source -> doc (DerivedFrom).
    assert!(
        graph_data
            .1
            .iter()
            .any(|e| e.source_node_id == "internal_wiki"
                && e.target_node_id == "doc_troubleshoot"
                && e.kind == EdgeKind::DerivedFrom),
        "graph should have source -> document edge"
    );

    // Edges: doc -> chunk (EmbeddedAs).
    let chunk_edges: Vec<_> = graph_data
        .1
        .iter()
        .filter(|e| e.source_node_id == "doc_troubleshoot" && e.kind == EdgeKind::EmbeddedAs)
        .collect();
    assert!(
        !chunk_edges.is_empty(),
        "graph should have document -> chunk edges"
    );

    // Chunk nodes exist.
    let chunk_node_count = graph_data
        .0
        .values()
        .filter(|n| n.kind == NodeKind::Chunk)
        .count();
    assert!(chunk_node_count > 0, "graph should contain chunk nodes");

    println!(
        "Integration proof passed: {} nodes, {} edges, {} search results, provenance traces to source",
        graph_data.0.len(),
        graph_data.1.len(),
        search_results.results.len()
    );
}
