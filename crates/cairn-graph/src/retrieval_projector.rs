use cairn_domain::{KnowledgeDocumentId, SourceId};

use crate::projections::{
    EdgeKind, GraphEdge, GraphNode, GraphProjection, GraphProjectionError, NodeKind,
};

/// Projects retrieval entities into graph structure for provenance queries.
///
/// Creates the source -> document -> chunk chain that enables the
/// RetrievalProvenance query family (answer -> chunk -> document -> source).
pub struct RetrievalGraphProjector<P: GraphProjection> {
    projection: P,
}

impl<P: GraphProjection> RetrievalGraphProjector<P> {
    pub fn new(projection: P) -> Self {
        Self { projection }
    }

    /// Record a source in the graph.
    pub async fn on_source_registered(
        &self,
        source_id: &SourceId,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_node(GraphNode {
                node_id: source_id.as_str().to_owned(),
                kind: NodeKind::Source,
                project: None,
                created_at: ts,
            })
            .await
    }

    /// Record a document, linked to its source.
    pub async fn on_document_ingested(
        &self,
        document_id: &KnowledgeDocumentId,
        source_id: &SourceId,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_node(GraphNode {
                node_id: document_id.as_str().to_owned(),
                kind: NodeKind::Document,
                project: None,
                created_at: ts,
            })
            .await?;

        self.projection
            .add_edge(GraphEdge {
                source_node_id: source_id.as_str().to_owned(),
                target_node_id: document_id.as_str().to_owned(),
                kind: EdgeKind::DerivedFrom,
                created_at: ts,
            })
            .await
    }

    /// Record chunks from a document.
    pub async fn on_chunks_created(
        &self,
        chunk_ids: &[String],
        document_id: &KnowledgeDocumentId,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        for chunk_id in chunk_ids {
            self.projection
                .add_node(GraphNode {
                    node_id: chunk_id.clone(),
                    kind: NodeKind::Chunk,
                    project: None,
                    created_at: ts,
                })
                .await?;

            self.projection
                .add_edge(GraphEdge {
                    source_node_id: document_id.as_str().to_owned(),
                    target_node_id: chunk_id.clone(),
                    kind: EdgeKind::EmbeddedAs,
                    created_at: ts,
                })
                .await?;
        }
        Ok(())
    }

    /// Record a citation: an answer/output node cited a chunk.
    pub async fn on_chunk_cited(
        &self,
        answer_node_id: &str,
        chunk_id: &str,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_edge(GraphEdge {
                source_node_id: answer_node_id.to_owned(),
                target_node_id: chunk_id.to_owned(),
                kind: EdgeKind::Cited,
                created_at: ts,
            })
            .await
    }

    /// Record a retrieval read: a run/task read from a chunk.
    pub async fn on_chunk_read(
        &self,
        run_or_task_node_id: &str,
        chunk_id: &str,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_edge(GraphEdge {
                source_node_id: run_or_task_node_id.to_owned(),
                target_node_id: chunk_id.to_owned(),
                kind: EdgeKind::ReadFrom,
                created_at: ts,
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct MemGraph {
        nodes: Mutex<HashMap<String, GraphNode>>,
        edges: Mutex<Vec<GraphEdge>>,
    }

    impl MemGraph {
        fn new() -> Self {
            Self {
                nodes: Mutex::new(HashMap::new()),
                edges: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl GraphProjection for Arc<MemGraph> {
        async fn add_node(&self, node: GraphNode) -> Result<(), GraphProjectionError> {
            self.nodes
                .lock()
                .unwrap()
                .insert(node.node_id.clone(), node);
            Ok(())
        }
        async fn add_edge(&self, edge: GraphEdge) -> Result<(), GraphProjectionError> {
            self.edges.lock().unwrap().push(edge);
            Ok(())
        }
        async fn node_exists(&self, node_id: &str) -> Result<bool, GraphProjectionError> {
            Ok(self.nodes.lock().unwrap().contains_key(node_id))
        }
    }

    #[tokio::test]
    async fn projects_full_retrieval_provenance_chain() {
        let graph = Arc::new(MemGraph::new());
        let projector = RetrievalGraphProjector::new(graph.clone());

        let source_id = SourceId::new("src_docs");
        let doc_id = KnowledgeDocumentId::new("doc_1");

        // Source -> Document -> Chunks
        projector
            .on_source_registered(&source_id, 100)
            .await
            .unwrap();
        projector
            .on_document_ingested(&doc_id, &source_id, 200)
            .await
            .unwrap();
        projector
            .on_chunks_created(&["chunk_1".to_owned(), "chunk_2".to_owned()], &doc_id, 300)
            .await
            .unwrap();

        // Answer cites chunk_1, run reads chunk_2.
        projector
            .on_chunk_cited("answer_node", "chunk_1", 400)
            .await
            .unwrap();
        projector
            .on_chunk_read("run_1", "chunk_2", 400)
            .await
            .unwrap();

        let nodes = graph.nodes.lock().unwrap();
        assert_eq!(nodes.len(), 4); // source + doc + 2 chunks
        assert_eq!(nodes["src_docs"].kind, NodeKind::Source);
        assert_eq!(nodes["doc_1"].kind, NodeKind::Document);
        assert_eq!(nodes["chunk_1"].kind, NodeKind::Chunk);

        let edges = graph.edges.lock().unwrap();
        // derived_from (source->doc) + 2x embedded_as (doc->chunk) + cited + read_from = 5
        assert_eq!(edges.len(), 5);
        assert!(edges.iter().any(|e| e.kind == EdgeKind::DerivedFrom));
        assert!(edges.iter().any(|e| e.kind == EdgeKind::EmbeddedAs));
        assert!(edges.iter().any(|e| e.kind == EdgeKind::Cited));
        assert!(edges.iter().any(|e| e.kind == EdgeKind::ReadFrom));
    }
}
