//! Concrete graph expansion hook for graph-assisted deep search (RFC 003/004).
//!
//! Uses GraphQueryService::neighbors to find related documents via graph edges,
//! returning their node IDs as additional query strings for the next retrieval hop.

use async_trait::async_trait;
use cairn_graph::projections::{EdgeKind, NodeKind};
use cairn_graph::queries::{GraphQueryService, TraversalDirection};

use crate::deep_search_impl::GraphExpansionHook;
use crate::retrieval::RetrievalResult;

/// Graph-backed expansion that finds related documents through graph edges.
///
/// For each retrieval result, looks up the chunk's document in the graph
/// and follows DerivedFrom/Cited/ReadFrom edges to find related documents,
/// returning their node IDs as additional queries for the next hop.
pub struct GraphBackedExpansion<Q: GraphQueryService> {
    query_service: Q,
    max_expansions: usize,
}

impl<Q: GraphQueryService> GraphBackedExpansion<Q> {
    pub fn new(query_service: Q) -> Self {
        Self {
            query_service,
            max_expansions: 5,
        }
    }

    pub fn with_max_expansions(mut self, max: usize) -> Self {
        self.max_expansions = max;
        self
    }
}

/// Edge kinds that indicate meaningful document relationships for retrieval expansion.
const EXPANSION_EDGES: &[EdgeKind] = &[
    EdgeKind::DerivedFrom,
    EdgeKind::Cited,
    EdgeKind::ReadFrom,
    EdgeKind::EmbeddedAs,
];

#[async_trait]
impl<Q: GraphQueryService + 'static> GraphExpansionHook for GraphBackedExpansion<Q> {
    async fn expand(&self, _query: &str, results: &[RetrievalResult]) -> Vec<String> {
        let mut expansions = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for result in results {
            if expansions.len() >= self.max_expansions {
                break;
            }

            let doc_id = result.chunk.document_id.as_str();
            if seen.contains(doc_id) {
                continue;
            }
            seen.insert(doc_id.to_owned());

            // Follow expansion edges from this document to find related nodes.
            for edge_kind in EXPANSION_EDGES {
                if expansions.len() >= self.max_expansions {
                    break;
                }

                let neighbors = self
                    .query_service
                    .neighbors(
                        doc_id,
                        Some(*edge_kind),
                        TraversalDirection::Upstream,
                        3,
                    )
                    .await;

                if let Ok(neighbors) = neighbors {
                    for (_, node) in neighbors {
                        if node.kind == NodeKind::Document || node.kind == NodeKind::Source {
                            if seen.insert(node.node_id.clone()) {
                                expansions.push(node.node_id);
                                if expansions.len() >= self.max_expansions {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        expansions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{ChunkRecord, SourceType};
    use crate::retrieval::{RetrievalResult, ScoringBreakdown};
    use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};
    use cairn_graph::projections::{GraphEdge, GraphNode, GraphProjectionError};
    use cairn_graph::queries::{GraphQuery, GraphQueryError, Subgraph};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct TestQueryService {
        neighbors: Mutex<HashMap<String, Vec<(GraphEdge, GraphNode)>>>,
    }

    impl TestQueryService {
        fn new() -> Self {
            Self {
                neighbors: Mutex::new(HashMap::new()),
            }
        }

        fn add_neighbor(&self, from: &str, edge: GraphEdge, node: GraphNode) {
            self.neighbors
                .lock()
                .unwrap()
                .entry(from.to_owned())
                .or_default()
                .push((edge, node));
        }
    }

    #[async_trait]
    impl GraphQueryService for TestQueryService {
        async fn query(&self, _query: GraphQuery) -> Result<Subgraph, GraphQueryError> {
            Ok(Subgraph {
                nodes: vec![],
                edges: vec![],
            })
        }

        async fn neighbors(
            &self,
            node_id: &str,
            _edge_filter: Option<EdgeKind>,
            _direction: TraversalDirection,
            _limit: usize,
        ) -> Result<Vec<(GraphEdge, GraphNode)>, GraphQueryError> {
            let neighbors = self.neighbors.lock().unwrap();
            Ok(neighbors.get(node_id).cloned().unwrap_or_default())
        }
    }

    fn make_result(doc_id: &str) -> RetrievalResult {
        RetrievalResult {
            chunk: ChunkRecord {
                chunk_id: ChunkId::new(format!("{doc_id}_0")),
                document_id: KnowledgeDocumentId::new(doc_id),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                text: "test".to_owned(),
                position: 0,
                created_at: 0,
                updated_at: None,
                provenance_metadata: None,
                credibility_score: None,
                graph_linkage: None,
                embedding: None,
                content_hash: None,
            },
            score: 1.0,
            breakdown: ScoringBreakdown::default(),
        }
    }

    #[tokio::test]
    async fn expands_via_graph_neighbors() {
        let svc = TestQueryService::new();
        svc.add_neighbor(
            "doc_1",
            GraphEdge {
                source_node_id: "doc_related".to_owned(),
                target_node_id: "doc_1".to_owned(),
                kind: EdgeKind::DerivedFrom,
                created_at: 0,
            },
            GraphNode {
                node_id: "doc_related".to_owned(),
                kind: NodeKind::Document,
                project: None,
                created_at: 0,
            },
        );

        let hook = GraphBackedExpansion::new(svc);
        let results = vec![make_result("doc_1")];
        let expansions = hook.expand("test query", &results).await;

        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0], "doc_related");
    }

    #[tokio::test]
    async fn respects_max_expansions() {
        let svc = TestQueryService::new();
        for i in 0..10 {
            svc.add_neighbor(
                "doc_1",
                GraphEdge {
                    source_node_id: format!("related_{i}"),
                    target_node_id: "doc_1".to_owned(),
                    kind: EdgeKind::Cited,
                    created_at: 0,
                },
                GraphNode {
                    node_id: format!("related_{i}"),
                    kind: NodeKind::Document,
                    project: None,
                    created_at: 0,
                },
            );
        }

        let hook = GraphBackedExpansion::new(svc).with_max_expansions(3);
        let results = vec![make_result("doc_1")];
        let expansions = hook.expand("test", &results).await;

        assert!(expansions.len() <= 3);
    }

    #[tokio::test]
    async fn no_expansions_without_graph_neighbors() {
        let svc = TestQueryService::new();
        let hook = GraphBackedExpansion::new(svc);
        let results = vec![make_result("doc_1")];
        let expansions = hook.expand("test", &results).await;

        assert!(expansions.is_empty());
    }

    #[tokio::test]
    async fn deduplicates_across_results() {
        let svc = TestQueryService::new();
        // Both doc_1 and doc_2 link to same related doc.
        for doc in &["doc_1", "doc_2"] {
            svc.add_neighbor(
                doc,
                GraphEdge {
                    source_node_id: "shared_related".to_owned(),
                    target_node_id: doc.to_string(),
                    kind: EdgeKind::DerivedFrom,
                    created_at: 0,
                },
                GraphNode {
                    node_id: "shared_related".to_owned(),
                    kind: NodeKind::Document,
                    project: None,
                    created_at: 0,
                },
            );
        }

        let hook = GraphBackedExpansion::new(svc);
        let results = vec![make_result("doc_1"), make_result("doc_2")];
        let expansions = hook.expand("test", &results).await;

        // Should deduplicate: only one "shared_related".
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0], "shared_related");
    }
}
