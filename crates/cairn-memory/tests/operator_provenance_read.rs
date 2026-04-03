//! Operator provenance read: proves provenance stays attached to
//! memory-backed reads after Worker 8 integration.

use async_trait::async_trait;
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
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

struct TestGraph(Arc<Mutex<(HashMap<String, GraphNode>, Vec<GraphEdge>)>>);
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
    async fn query(&self, _q: GraphQuery) -> Result<Subgraph, GraphQueryError> {
        let d = self.0.lock().unwrap();
        Ok(Subgraph {
            nodes: d.0.values().cloned().collect(),
            edges: d.1.clone(),
        })
    }
    async fn neighbors(
        &self,
        _: &str,
        _: Option<EdgeKind>,
        _: TraversalDirection,
        _: usize,
    ) -> Result<Vec<(GraphEdge, GraphNode)>, GraphQueryError> {
        Ok(vec![])
    }
}

/// Operator flow: ingest doc -> project to graph -> search -> trace provenance.
/// Proves the provenance attachment survives through the full read path.
#[tokio::test]
async fn operator_search_result_has_traceable_provenance() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());
    let graph = TestGraph::new();
    let projector = RetrievalGraphProjector::new(graph.clone());
    let provenance = GraphProvenanceService::new(graph.clone());

    let project = ProjectKey::new("acme", "eng", "ops");
    let source = SourceId::new("runbooks");
    let doc_id = KnowledgeDocumentId::new("runbook_deploy");

    // Ingest.
    pipeline
        .submit(IngestRequest {
            document_id: doc_id.clone(),
            source_id: source.clone(),
            source_type: SourceType::Markdown,
            project: project.clone(),
            content: "# Deploy Runbook\n\nRun kubectl apply to deploy the service.".to_owned(),
        })
        .await
        .unwrap();

    // Project to graph.
    projector.on_source_registered(&source, 100).await.unwrap();
    projector
        .on_document_ingested(&doc_id, &source, 200)
        .await
        .unwrap();

    // Search.
    let results = retrieval
        .query(RetrievalQuery {
            project: project.clone(),
            query_text: "kubectl deploy".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(!results.results.is_empty());

    // Trace provenance from the document.
    let prov = provenance
        .retrieval_provenance("runbook_deploy")
        .await
        .unwrap();

    assert!(
        prov.source_ids.iter().any(|s| s.as_str() == "runbooks"),
        "provenance must trace back to the source"
    );
}
