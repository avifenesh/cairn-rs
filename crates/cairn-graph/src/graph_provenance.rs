use async_trait::async_trait;
use cairn_domain::SessionId;

use crate::projections::NodeKind;
use crate::provenance::{
    ExecutionProvenance, ProvenanceChain, ProvenanceError, ProvenanceLink, ProvenanceService,
    RetrievalProvenance,
};
use crate::queries::{GraphQuery, GraphQueryService, TraversalDirection};

/// Graph-backed implementation of ProvenanceService.
///
/// Uses the GraphQueryService to traverse the graph and build
/// execution and retrieval provenance chains for operator surfaces.
pub struct GraphProvenanceService<Q: GraphQueryService> {
    query_service: Q,
}

impl<Q: GraphQueryService> GraphProvenanceService<Q> {
    pub fn new(query_service: Q) -> Self {
        Self { query_service }
    }
}

#[async_trait]
impl<Q: GraphQueryService + 'static> ProvenanceService for GraphProvenanceService<Q> {
    async fn execution_provenance(
        &self,
        session_id: &SessionId,
    ) -> Result<ExecutionProvenance, ProvenanceError> {
        let subgraph = self
            .query_service
            .query(GraphQuery::ExecutionTrace {
                root_node_id: session_id.as_str().to_owned(),
                root_kind: NodeKind::Session,
                max_depth: 10,
            })
            .await
            .map_err(|e| ProvenanceError::StorageError(e.to_string()))?;

        let mut run_ids = Vec::new();
        let mut task_ids = Vec::new();
        let mut tool_invocation_ids = Vec::new();
        let mut checkpoint_ids = Vec::new();
        let mut prompt_release_ids = Vec::new();

        for node in &subgraph.nodes {
            match node.kind {
                NodeKind::Run => run_ids.push(cairn_domain::RunId::new(&node.node_id)),
                NodeKind::Task => task_ids.push(cairn_domain::TaskId::new(&node.node_id)),
                NodeKind::ToolInvocation => {
                    tool_invocation_ids.push(cairn_domain::ToolInvocationId::new(&node.node_id));
                }
                NodeKind::Checkpoint => {
                    checkpoint_ids.push(cairn_domain::CheckpointId::new(&node.node_id));
                }
                NodeKind::PromptRelease => {
                    prompt_release_ids.push(cairn_domain::PromptReleaseId::new(&node.node_id));
                }
                _ => {}
            }
        }

        Ok(ExecutionProvenance {
            session_id: session_id.clone(),
            run_ids,
            task_ids,
            tool_invocation_ids,
            checkpoint_ids,
            prompt_release_ids,
        })
    }

    async fn retrieval_provenance(
        &self,
        answer_node_id: &str,
    ) -> Result<RetrievalProvenance, ProvenanceError> {
        let subgraph = self
            .query_service
            .query(GraphQuery::RetrievalProvenance {
                answer_node_id: answer_node_id.to_owned(),
            })
            .await
            .map_err(|e| ProvenanceError::StorageError(e.to_string()))?;

        let mut source_ids = Vec::new();
        let mut document_ids = Vec::new();
        let mut chunk_ids = Vec::new();

        for node in &subgraph.nodes {
            match node.kind {
                NodeKind::Source => {
                    source_ids.push(cairn_domain::SourceId::new(&node.node_id));
                }
                NodeKind::Document => {
                    document_ids.push(cairn_domain::KnowledgeDocumentId::new(&node.node_id));
                }
                NodeKind::Chunk => {
                    chunk_ids.push(node.node_id.clone());
                }
                _ => {}
            }
        }

        Ok(RetrievalProvenance {
            source_ids,
            document_ids,
            chunk_ids,
        })
    }

    async fn provenance_chain(
        &self,
        node_id: &str,
        max_depth: u32,
    ) -> Result<ProvenanceChain, ProvenanceError> {
        let subgraph = self
            .query_service
            .query(GraphQuery::DependencyPath {
                node_id: node_id.to_owned(),
                direction: TraversalDirection::Upstream,
                max_depth,
            })
            .await
            .map_err(|e| ProvenanceError::StorageError(e.to_string()))?;

        // Build links from the traversal, computing depth from edges.
        let mut links = Vec::new();
        for node in &subgraph.nodes {
            if node.node_id == node_id {
                continue; // Skip the root itself.
            }
            // Approximate depth by counting edges from root.
            let depth = compute_depth(node_id, &node.node_id, &subgraph.edges);
            links.push(ProvenanceLink {
                node_id: node.node_id.clone(),
                kind: node.kind,
                depth,
            });
        }

        links.sort_by_key(|l| l.depth);

        Ok(ProvenanceChain {
            root_node_id: node_id.to_owned(),
            links,
        })
    }
}

/// Approximate depth by counting the shortest edge path from target to source.
fn compute_depth(root_id: &str, target_id: &str, edges: &[crate::projections::GraphEdge]) -> u32 {
    // Simple BFS from target back to root through edges.
    let mut frontier = vec![target_id.to_owned()];
    let mut visited = std::collections::HashSet::new();
    let mut depth = 1u32;

    for _ in 0..20 {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for node in &frontier {
            if visited.contains(node.as_str()) {
                continue;
            }
            visited.insert(node.clone());
            if node == root_id {
                return depth - 1;
            }
            for edge in edges {
                if edge.target_node_id == *node {
                    next.push(edge.source_node_id.clone());
                }
                if edge.source_node_id == *node {
                    next.push(edge.target_node_id.clone());
                }
            }
        }
        frontier = next;
        depth += 1;
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_projector::EventProjector;
    use crate::projections::{
        EdgeKind, GraphEdge, GraphNode, GraphProjection, GraphProjectionError,
    };
    use async_trait::async_trait;
    use cairn_domain::*;
    use cairn_store::event_log::StoredEvent;
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

    #[async_trait]
    impl GraphQueryService for Arc<MemGraph> {
        async fn query(
            &self,
            _query: GraphQuery,
        ) -> Result<crate::queries::Subgraph, crate::queries::GraphQueryError> {
            let nodes: Vec<GraphNode> = self.nodes.lock().unwrap().values().cloned().collect();
            let edges: Vec<GraphEdge> = self.edges.lock().unwrap().clone();
            Ok(crate::queries::Subgraph { nodes, edges })
        }

        async fn neighbors(
            &self,
            _node_id: &str,
            _edge_filter: Option<EdgeKind>,
            _direction: TraversalDirection,
            _limit: usize,
        ) -> Result<Vec<(GraphEdge, GraphNode)>, crate::queries::GraphQueryError> {
            Ok(vec![])
        }

        async fn find_edges_by_source(
            &self,
            _source_node_id: &str,
            _edge_filter: Option<EdgeKind>,
            _limit: usize,
        ) -> Result<Vec<GraphEdge>, crate::queries::GraphQueryError> {
            Ok(vec![])
        }

        async fn find_edges_by_target(
            &self,
            _target_node_id: &str,
            _edge_filter: Option<EdgeKind>,
            _limit: usize,
        ) -> Result<Vec<GraphEdge>, crate::queries::GraphQueryError> {
            Ok(vec![])
        }

        async fn shortest_path(
            &self,
            _from_node_id: &str,
            _to_node_id: &str,
            _edge_filter: Option<EdgeKind>,
            _max_depth: u32,
        ) -> Result<Option<crate::queries::Subgraph>, crate::queries::GraphQueryError> {
            Ok(None)
        }
    }

    fn make_stored(payload: RuntimeEvent) -> StoredEvent {
        StoredEvent {
            position: cairn_store::EventPosition(1),
            envelope: EventEnvelope::for_runtime_event(
                EventId::new("evt_1"),
                EventSource::Runtime,
                payload,
            ),
            stored_at: 1000,
        }
    }

    #[tokio::test]
    async fn execution_provenance_finds_runs_and_tasks() {
        let graph = Arc::new(MemGraph::new());
        let projector = EventProjector::new(graph.clone());

        projector
            .project_events(&[
                make_stored(RuntimeEvent::SessionCreated(SessionCreated {
                    project: ProjectKey::new("t", "w", "p"),
                    session_id: SessionId::new("sess_1"),
                })),
                make_stored(RuntimeEvent::RunCreated(RunCreated {
                    project: ProjectKey::new("t", "w", "p"),
                    session_id: SessionId::new("sess_1"),
                    run_id: RunId::new("run_1"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                })),
                make_stored(RuntimeEvent::TaskCreated(TaskCreated {
                    project: ProjectKey::new("t", "w", "p"),
                    task_id: TaskId::new("task_1"),
                    parent_run_id: Some(RunId::new("run_1")),
                    parent_task_id: None,
                    prompt_release_id: None,
                    session_id: None,
                })),
            ])
            .await
            .unwrap();

        let prov_service = GraphProvenanceService::new(graph.clone());
        let prov = prov_service
            .execution_provenance(&SessionId::new("sess_1"))
            .await
            .unwrap();

        assert_eq!(prov.session_id, SessionId::new("sess_1"));
        assert!(prov.run_ids.contains(&RunId::new("run_1")));
        assert!(prov.task_ids.contains(&TaskId::new("task_1")));
    }

    #[tokio::test]
    async fn provenance_chain_traverses_upstream() {
        let graph = Arc::new(MemGraph::new());
        let projector = EventProjector::new(graph.clone());

        projector
            .project_events(&[
                make_stored(RuntimeEvent::SessionCreated(SessionCreated {
                    project: ProjectKey::new("t", "w", "p"),
                    session_id: SessionId::new("s1"),
                })),
                make_stored(RuntimeEvent::RunCreated(RunCreated {
                    project: ProjectKey::new("t", "w", "p"),
                    session_id: SessionId::new("s1"),
                    run_id: RunId::new("r1"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                })),
            ])
            .await
            .unwrap();

        let prov_service = GraphProvenanceService::new(graph.clone());
        let chain = prov_service.provenance_chain("r1", 5).await.unwrap();

        assert_eq!(chain.root_node_id, "r1");
        assert!(!chain.links.is_empty());
    }
}
