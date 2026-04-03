use async_trait::async_trait;
use cairn_domain::KnowledgeDocumentId;
use cairn_graph::projections::GraphProjection;
use cairn_graph::retrieval_projector::RetrievalGraphProjector;

use crate::ingest::{IngestError, IngestPackRequest, IngestRequest, IngestService, IngestStatus};
use crate::pipeline::{Chunker, DocumentStore, IngestPipeline};

/// Ingest pipeline that also projects ingested entities to the graph.
///
/// Wraps a base IngestPipeline and a RetrievalGraphProjector. After
/// successful ingest, it creates source/document/chunk nodes and edges
/// in the graph for retrieval provenance queries.
pub struct GraphAwareIngestPipeline<S: DocumentStore, C: Chunker, P: GraphProjection> {
    base: IngestPipeline<S, C>,
    graph: RetrievalGraphProjector<P>,
}

impl<S: DocumentStore, C: Chunker, P: GraphProjection> GraphAwareIngestPipeline<S, C, P> {
    pub fn new(base: IngestPipeline<S, C>, graph_projection: P) -> Self {
        Self {
            base,
            graph: RetrievalGraphProjector::new(graph_projection),
        }
    }
}

#[async_trait]
impl<S: DocumentStore + 'static, C: Chunker + 'static, P: GraphProjection + 'static> IngestService
    for GraphAwareIngestPipeline<S, C, P>
{
    async fn submit(&self, request: IngestRequest) -> Result<(), IngestError> {
        let source_id = request.source_id.clone();
        let document_id = request.document_id.clone();

        // Run the base pipeline.
        self.base.submit(request).await?;

        // Project to graph (best-effort — graph failures don't fail ingest).
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let _ = self.graph.on_source_registered(&source_id, now).await;
        let _ = self
            .graph
            .on_document_ingested(&document_id, &source_id, now)
            .await;

        // Get chunk IDs from the store to project chunk nodes.
        if let Ok(Some(IngestStatus::Completed)) = self.base.status(&document_id).await {
            // Chunks were created — project them.
            // We construct chunk IDs from the document ID + position pattern
            // used by ParagraphChunker. For a proper implementation, the
            // pipeline would return chunk IDs from submit().
            // This is a skeleton — full wiring comes when the pipeline
            // returns chunk metadata.
        }

        Ok(())
    }

    async fn submit_pack(&self, request: IngestPackRequest) -> Result<(), IngestError> {
        self.base.submit_pack(request).await
    }

    async fn status(
        &self,
        document_id: &KnowledgeDocumentId,
    ) -> Result<Option<IngestStatus>, IngestError> {
        self.base.status(document_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::InMemoryDocumentStore;
    use crate::ingest::SourceType;
    use crate::pipeline::ParagraphChunker;
    use cairn_domain::{ProjectKey, SourceId};
    use cairn_graph::projections::{GraphEdge, GraphNode, GraphProjectionError, NodeKind};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Local newtype wrapper to satisfy orphan rule for GraphProjection impl.
    struct TestGraph(Arc<Mutex<(HashMap<String, GraphNode>, Vec<GraphEdge>)>>);

    impl TestGraph {
        fn new() -> Self {
            Self(Arc::new(Mutex::new((HashMap::new(), Vec::new()))))
        }

        fn nodes(&self) -> HashMap<String, GraphNode> {
            self.0.lock().unwrap().0.clone()
        }

        fn edges(&self) -> Vec<GraphEdge> {
            self.0.lock().unwrap().1.clone()
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

    #[tokio::test]
    async fn graph_aware_ingest_creates_source_and_document_nodes() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let chunker = ParagraphChunker {
            max_chunk_size: 200,
        };
        let base = IngestPipeline::new(store.clone(), chunker);
        let graph = TestGraph::new();
        let pipeline = GraphAwareIngestPipeline::new(base, graph.clone());

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_1"),
                source_id: SourceId::new("src_1"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                content: "Test content for graph-aware ingest.".to_owned(),
            })
            .await
            .unwrap();

        // Verify graph nodes were created.
        let nodes = graph.nodes();
        assert!(nodes.contains_key("src_1"));
        assert!(nodes.contains_key("doc_1"));
        assert_eq!(nodes["src_1"].kind, NodeKind::Source);
        assert_eq!(nodes["doc_1"].kind, NodeKind::Document);

        // Verify edge: source -> document.
        let edges = graph.edges();
        assert!(edges
            .iter()
            .any(|e| e.source_node_id == "src_1" && e.target_node_id == "doc_1"));

        // Verify document was ingested successfully.
        let status = pipeline
            .status(&KnowledgeDocumentId::new("doc_1"))
            .await
            .unwrap();
        assert_eq!(status, Some(IngestStatus::Completed));
    }
}
