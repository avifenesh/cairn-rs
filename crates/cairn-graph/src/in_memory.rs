//! In-memory graph store for testing and local-mode use.
//!
//! Implements both `GraphProjection` (write) and `GraphQueryService` (read)
//! with per-variant traversal logic for the six RFC 004 query families.

use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::projections::{
    EdgeKind, GraphEdge, GraphNode, GraphProjection, GraphProjectionError,
};
use crate::queries::{
    GraphQuery, GraphQueryError, GraphQueryService, Subgraph, TraversalDirection,
};

/// In-memory graph store backed by HashMaps under a Mutex.
pub struct InMemoryGraphStore {
    nodes: Mutex<HashMap<String, GraphNode>>,
    edges: Mutex<Vec<GraphEdge>>,
}

impl InMemoryGraphStore {
    pub fn new() -> Self {
        Self {
            nodes: Mutex::new(HashMap::new()),
            edges: Mutex::new(Vec::new()),
        }
    }

    /// Snapshot of all nodes (for testing).
    pub fn all_nodes(&self) -> HashMap<String, GraphNode> {
        self.nodes.lock().unwrap().clone()
    }

    /// Snapshot of all edges (for testing).
    pub fn all_edges(&self) -> Vec<GraphEdge> {
        self.edges.lock().unwrap().clone()
    }
}

impl Default for InMemoryGraphStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GraphProjection for std::sync::Arc<InMemoryGraphStore> {
    async fn add_node(&self, node: GraphNode) -> Result<(), GraphProjectionError> {
        (**self).add_node(node).await
    }

    async fn add_edge(&self, edge: GraphEdge) -> Result<(), GraphProjectionError> {
        (**self).add_edge(edge).await
    }

    async fn node_exists(&self, node_id: &str) -> Result<bool, GraphProjectionError> {
        (**self).node_exists(node_id).await
    }
}

#[async_trait]
impl GraphProjection for InMemoryGraphStore {
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
impl GraphQueryService for InMemoryGraphStore {
    async fn query(&self, query: GraphQuery) -> Result<Subgraph, GraphQueryError> {
        let nodes = self.nodes.lock().unwrap().clone();
        let edges = self.edges.lock().unwrap().clone();

        match query {
            GraphQuery::ExecutionTrace {
                root_node_id,
                max_depth,
                ..
            } => {
                // Follow Triggered, Spawned, UsedTool, SentTo edges in both
                // directions. Edge direction in the projector is mixed
                // (run→session for Triggered, run→task for Spawned), so
                // bidirectional traversal is needed.
                let exec_edges: HashSet<EdgeKind> = [
                    EdgeKind::Triggered,
                    EdgeKind::Spawned,
                    EdgeKind::UsedTool,
                    EdgeKind::SentTo,
                ]
                .into_iter()
                .collect();
                Ok(bfs_bidirectional(
                    &root_node_id,
                    max_depth,
                    &nodes,
                    &edges,
                    Some(&exec_edges),
                ))
            }

            GraphQuery::DependencyPath {
                node_id,
                direction,
                max_depth,
            } => {
                let dep_edges: HashSet<EdgeKind> = [
                    EdgeKind::DependedOn,
                    EdgeKind::Spawned,
                    EdgeKind::ResumedFrom,
                ]
                .into_iter()
                .collect();
                match direction {
                    TraversalDirection::Downstream => Ok(bfs_downstream(
                        &node_id,
                        max_depth,
                        &nodes,
                        &edges,
                        Some(&dep_edges),
                    )),
                    TraversalDirection::Upstream => Ok(bfs_upstream(
                        &node_id,
                        max_depth,
                        &nodes,
                        &edges,
                        Some(&dep_edges),
                    )),
                }
            }

            GraphQuery::PromptProvenance { outcome_node_id } => {
                // Upstream traversal following UsedPrompt, DerivedFrom, ReleasedAs.
                let prompt_edges: HashSet<EdgeKind> = [
                    EdgeKind::UsedPrompt,
                    EdgeKind::DerivedFrom,
                    EdgeKind::ReleasedAs,
                ]
                .into_iter()
                .collect();
                Ok(bfs_upstream(
                    &outcome_node_id,
                    10,
                    &nodes,
                    &edges,
                    Some(&prompt_edges),
                ))
            }

            GraphQuery::RetrievalProvenance { answer_node_id } => {
                // Upstream traversal following Cited, ReadFrom, EmbeddedAs, DerivedFrom.
                let retrieval_edges: HashSet<EdgeKind> = [
                    EdgeKind::Cited,
                    EdgeKind::ReadFrom,
                    EdgeKind::EmbeddedAs,
                    EdgeKind::DerivedFrom,
                ]
                .into_iter()
                .collect();
                Ok(bfs_upstream(
                    &answer_node_id,
                    5,
                    &nodes,
                    &edges,
                    Some(&retrieval_edges),
                ))
            }

            GraphQuery::DecisionInvolvement { decision_node_id } => {
                // Upstream traversal following UsedTool, UsedPrompt, ApprovedBy.
                let decision_edges: HashSet<EdgeKind> = [
                    EdgeKind::UsedTool,
                    EdgeKind::UsedPrompt,
                    EdgeKind::ApprovedBy,
                ]
                .into_iter()
                .collect();
                Ok(bfs_upstream(
                    &decision_node_id,
                    5,
                    &nodes,
                    &edges,
                    Some(&decision_edges),
                ))
            }

            GraphQuery::EvalLineage { eval_run_node_id } => {
                // Upstream traversal following EvaluatedBy, ReleasedAs, DerivedFrom.
                let eval_edges: HashSet<EdgeKind> = [
                    EdgeKind::EvaluatedBy,
                    EdgeKind::ReleasedAs,
                    EdgeKind::DerivedFrom,
                ]
                .into_iter()
                .collect();
                Ok(bfs_upstream(
                    &eval_run_node_id,
                    10,
                    &nodes,
                    &edges,
                    Some(&eval_edges),
                ))
            }
        }
    }

    async fn neighbors(
        &self,
        node_id: &str,
        edge_filter: Option<EdgeKind>,
        direction: TraversalDirection,
        limit: usize,
    ) -> Result<Vec<(GraphEdge, GraphNode)>, GraphQueryError> {
        let nodes = self.nodes.lock().unwrap();
        let edges = self.edges.lock().unwrap();

        let mut results = Vec::new();

        for edge in edges.iter() {
            if let Some(kind) = edge_filter {
                if edge.kind != kind {
                    continue;
                }
            }

            let neighbor_id = match direction {
                TraversalDirection::Downstream => {
                    if edge.source_node_id == node_id {
                        &edge.target_node_id
                    } else {
                        continue;
                    }
                }
                TraversalDirection::Upstream => {
                    if edge.target_node_id == node_id {
                        &edge.source_node_id
                    } else {
                        continue;
                    }
                }
            };

            if let Some(node) = nodes.get(neighbor_id) {
                results.push((edge.clone(), node.clone()));
            }

            if results.len() >= limit {
                break;
            }
        }

        Ok(results)
    }
}

/// BFS bidirectional: follow edges in both directions from root, optionally filtered by edge kind.
/// Used for execution traces where edge directions are mixed.
fn bfs_bidirectional(
    root_id: &str,
    max_depth: u32,
    nodes: &HashMap<String, GraphNode>,
    edges: &[GraphEdge],
    edge_filter: Option<&HashSet<EdgeKind>>,
) -> Subgraph {
    let mut result_nodes = Vec::new();
    let mut seen_edges = HashSet::new();
    let mut result_edges = Vec::new();
    let mut visited = HashSet::new();
    let mut frontier = vec![root_id.to_owned()];

    for _depth in 0..max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for nid in &frontier {
            if !visited.insert(nid.clone()) {
                continue;
            }
            if let Some(node) = nodes.get(nid.as_str()) {
                result_nodes.push(node.clone());
            }
            for (ei, edge) in edges.iter().enumerate() {
                if let Some(filter) = edge_filter {
                    if !filter.contains(&edge.kind) {
                        continue;
                    }
                }
                if edge.source_node_id == *nid {
                    if seen_edges.insert(ei) {
                        result_edges.push(edge.clone());
                    }
                    next.push(edge.target_node_id.clone());
                } else if edge.target_node_id == *nid {
                    if seen_edges.insert(ei) {
                        result_edges.push(edge.clone());
                    }
                    next.push(edge.source_node_id.clone());
                }
            }
        }
        frontier = next;
    }

    Subgraph {
        nodes: result_nodes,
        edges: result_edges,
    }
}

/// BFS downstream: follow outgoing edges from root, optionally filtered by edge kind.
fn bfs_downstream(
    root_id: &str,
    max_depth: u32,
    nodes: &HashMap<String, GraphNode>,
    edges: &[GraphEdge],
    edge_filter: Option<&HashSet<EdgeKind>>,
) -> Subgraph {
    let mut result_nodes = Vec::new();
    let mut seen_edges: HashSet<usize> = HashSet::new();
    let mut result_edges = Vec::new();
    let mut visited = HashSet::new();
    let mut frontier = vec![root_id.to_owned()];

    for _depth in 0..max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for nid in &frontier {
            if !visited.insert(nid.clone()) {
                continue;
            }
            if let Some(node) = nodes.get(nid.as_str()) {
                result_nodes.push(node.clone());
            }
            for (ei, edge) in edges.iter().enumerate() {
                if edge.source_node_id == *nid {
                    if let Some(filter) = edge_filter {
                        if !filter.contains(&edge.kind) {
                            continue;
                        }
                    }
                    if seen_edges.insert(ei) {
                        result_edges.push(edge.clone());
                    }
                    next.push(edge.target_node_id.clone());
                }
            }
        }
        frontier = next;
    }

    Subgraph {
        nodes: result_nodes,
        edges: result_edges,
    }
}

/// BFS upstream: follow incoming edges from leaf, optionally filtered by edge kind.
fn bfs_upstream(
    leaf_id: &str,
    max_depth: u32,
    nodes: &HashMap<String, GraphNode>,
    edges: &[GraphEdge],
    edge_filter: Option<&HashSet<EdgeKind>>,
) -> Subgraph {
    let mut result_nodes = Vec::new();
    let mut seen_edges: HashSet<usize> = HashSet::new();
    let mut result_edges = Vec::new();
    let mut visited = HashSet::new();
    let mut frontier = vec![leaf_id.to_owned()];

    for _depth in 0..max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for nid in &frontier {
            if !visited.insert(nid.clone()) {
                continue;
            }
            if let Some(node) = nodes.get(nid.as_str()) {
                result_nodes.push(node.clone());
            }
            for (ei, edge) in edges.iter().enumerate() {
                if edge.target_node_id == *nid {
                    if let Some(filter) = edge_filter {
                        if !filter.contains(&edge.kind) {
                            continue;
                        }
                    }
                    if seen_edges.insert(ei) {
                        result_edges.push(edge.clone());
                    }
                    next.push(edge.source_node_id.clone());
                }
            }
        }
        frontier = next;
    }

    Subgraph {
        nodes: result_nodes,
        edges: result_edges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_projector::EventProjector;
    use crate::projections::NodeKind;
    use cairn_domain::*;
    use cairn_store::event_log::StoredEvent;
    use std::sync::Arc;

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
    async fn execution_trace_follows_triggered_and_spawned() {
        let store = Arc::new(InMemoryGraphStore::new());
        let projector = EventProjector::new(store.clone());

        projector
            .project_events(&[
                make_stored(RuntimeEvent::SessionCreated(SessionCreated {
                    project: ProjectKey::new("t", "w", "p"),
                    session_id: SessionId::new("sess"),
                })),
                make_stored(RuntimeEvent::RunCreated(RunCreated {
                    project: ProjectKey::new("t", "w", "p"),
                    session_id: SessionId::new("sess"),
                    run_id: RunId::new("run"),
                    parent_run_id: None,
                    prompt_release_id: None,
                })),
                make_stored(RuntimeEvent::TaskCreated(TaskCreated {
                    project: ProjectKey::new("t", "w", "p"),
                    task_id: TaskId::new("task"),
                    parent_run_id: Some(RunId::new("run")),
                    parent_task_id: None,
                    prompt_release_id: None,
                })),
            ])
            .await
            .unwrap();

        let subgraph = store
            .query(GraphQuery::ExecutionTrace {
                root_node_id: "sess".to_owned(),
                root_kind: NodeKind::Session,
                max_depth: 5,
            })
            .await
            .unwrap();

        let node_ids: Vec<&str> = subgraph.nodes.iter().map(|n| n.node_id.as_str()).collect();
        assert!(node_ids.contains(&"sess"));
        assert!(node_ids.contains(&"run"));
        assert!(node_ids.contains(&"task"));
        assert_eq!(subgraph.nodes.len(), 3);
    }

    #[tokio::test]
    async fn dependency_path_upstream() {
        let store = Arc::new(InMemoryGraphStore::new());
        let projector = EventProjector::new(store.clone());

        projector
            .project_events(&[
                make_stored(RuntimeEvent::TaskCreated(TaskCreated {
                    project: ProjectKey::new("t", "w", "p"),
                    task_id: TaskId::new("parent_task"),
                    parent_run_id: None,
                    parent_task_id: None,
                    prompt_release_id: None,
                })),
                make_stored(RuntimeEvent::TaskCreated(TaskCreated {
                    project: ProjectKey::new("t", "w", "p"),
                    task_id: TaskId::new("child_task"),
                    parent_run_id: None,
                    parent_task_id: Some(TaskId::new("parent_task")),
                    prompt_release_id: None,
                })),
            ])
            .await
            .unwrap();

        let subgraph = store
            .query(GraphQuery::DependencyPath {
                node_id: "child_task".to_owned(),
                direction: TraversalDirection::Upstream,
                max_depth: 5,
            })
            .await
            .unwrap();

        let node_ids: Vec<&str> = subgraph.nodes.iter().map(|n| n.node_id.as_str()).collect();
        assert!(node_ids.contains(&"child_task"));
        assert!(node_ids.contains(&"parent_task"));
    }

    #[tokio::test]
    async fn neighbors_filters_by_edge_kind() {
        let store = Arc::new(InMemoryGraphStore::new());

        store
            .add_node(GraphNode {
                node_id: "a".to_owned(),
                kind: NodeKind::Run,
                project: None,
                created_at: 1,
            })
            .await
            .unwrap();
        store
            .add_node(GraphNode {
                node_id: "b".to_owned(),
                kind: NodeKind::Task,
                project: None,
                created_at: 2,
            })
            .await
            .unwrap();
        store
            .add_node(GraphNode {
                node_id: "c".to_owned(),
                kind: NodeKind::ToolInvocation,
                project: None,
                created_at: 3,
            })
            .await
            .unwrap();
        store
            .add_edge(GraphEdge {
                source_node_id: "a".to_owned(),
                target_node_id: "b".to_owned(),
                kind: EdgeKind::Spawned,
                created_at: 10,
            })
            .await
            .unwrap();
        store
            .add_edge(GraphEdge {
                source_node_id: "a".to_owned(),
                target_node_id: "c".to_owned(),
                kind: EdgeKind::UsedTool,
                created_at: 11,
            })
            .await
            .unwrap();

        // All neighbors downstream from a.
        let all = store
            .neighbors("a", None, TraversalDirection::Downstream, 10)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);

        // Only Spawned neighbors.
        let spawned = store
            .neighbors("a", Some(EdgeKind::Spawned), TraversalDirection::Downstream, 10)
            .await
            .unwrap();
        assert_eq!(spawned.len(), 1);
        assert_eq!(spawned[0].1.node_id, "b");

        // Upstream from b.
        let upstream = store
            .neighbors("b", None, TraversalDirection::Upstream, 10)
            .await
            .unwrap();
        assert_eq!(upstream.len(), 1);
        assert_eq!(upstream[0].1.node_id, "a");
    }

    #[tokio::test]
    async fn empty_graph_returns_empty_subgraph() {
        let store = InMemoryGraphStore::new();

        let subgraph = store
            .query(GraphQuery::ExecutionTrace {
                root_node_id: "nonexistent".to_owned(),
                root_kind: NodeKind::Session,
                max_depth: 5,
            })
            .await
            .unwrap();

        assert!(subgraph.nodes.is_empty());
        assert!(subgraph.edges.is_empty());
    }

    #[tokio::test]
    async fn graph_node_carries_project_scope() {
        let store = Arc::new(InMemoryGraphStore::new());
        let projector = EventProjector::new(store.clone());

        projector
            .project_events(&[make_stored(RuntimeEvent::SessionCreated(SessionCreated {
                project: ProjectKey::new("acme", "eng", "docs"),
                session_id: SessionId::new("scoped_sess"),
            }))])
            .await
            .unwrap();

        let nodes = store.all_nodes();
        let node = &nodes["scoped_sess"];
        assert_eq!(
            node.project.as_ref().unwrap().project_id.as_str(),
            "docs"
        );
    }
}
