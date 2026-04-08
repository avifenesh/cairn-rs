//! In-memory graph store for testing and local-mode use.
//!
//! Implements both `GraphProjection` (write) and `GraphQueryService` (read)
//! with per-variant traversal logic for the six RFC 004 query families.

use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::projections::{EdgeKind, GraphEdge, GraphNode, GraphProjection, GraphProjectionError};
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

            GraphQuery::MultiHop {
                start_node_id,
                max_hops,
                min_confidence,
                direction,
            } => match direction {
                TraversalDirection::Downstream => Ok(bfs_with_confidence(
                    &start_node_id,
                    max_hops,
                    min_confidence,
                    &nodes,
                    &edges,
                    BfsDirection::Downstream,
                )),
                TraversalDirection::Upstream => Ok(bfs_with_confidence(
                    &start_node_id,
                    max_hops,
                    min_confidence,
                    &nodes,
                    &edges,
                    BfsDirection::Upstream,
                )),
            },
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

    async fn find_edges_by_source(
        &self,
        source_node_id: &str,
        edge_filter: Option<EdgeKind>,
        limit: usize,
    ) -> Result<Vec<GraphEdge>, GraphQueryError> {
        let edges = self.edges.lock().unwrap();
        let mut results = Vec::new();
        for edge in edges.iter() {
            if edge.source_node_id == source_node_id {
                if let Some(kind) = edge_filter {
                    if edge.kind != kind {
                        continue;
                    }
                }
                results.push(edge.clone());
                if results.len() >= limit {
                    break;
                }
            }
        }
        Ok(results)
    }

    async fn find_edges_by_target(
        &self,
        target_node_id: &str,
        edge_filter: Option<EdgeKind>,
        limit: usize,
    ) -> Result<Vec<GraphEdge>, GraphQueryError> {
        let edges = self.edges.lock().unwrap();
        let mut results = Vec::new();
        for edge in edges.iter() {
            if edge.target_node_id == target_node_id {
                if let Some(kind) = edge_filter {
                    if edge.kind != kind {
                        continue;
                    }
                }
                results.push(edge.clone());
                if results.len() >= limit {
                    break;
                }
            }
        }
        Ok(results)
    }

    async fn shortest_path(
        &self,
        from_node_id: &str,
        to_node_id: &str,
        edge_filter: Option<EdgeKind>,
        max_depth: u32,
    ) -> Result<Option<Subgraph>, GraphQueryError> {
        let nodes = self.nodes.lock().unwrap().clone();
        let edges = self.edges.lock().unwrap().clone();

        if from_node_id == to_node_id {
            if let Some(node) = nodes.get(from_node_id) {
                return Ok(Some(Subgraph {
                    nodes: vec![node.clone()],
                    edges: vec![],
                }));
            }
            return Ok(None);
        }

        Ok(bfs_shortest_path(
            from_node_id,
            to_node_id,
            max_depth,
            &nodes,
            &edges,
            edge_filter,
        ))
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

/// Direction hint for the confidence-aware BFS.
enum BfsDirection {
    Downstream,
    Upstream,
}

/// BFS traversal with optional confidence filtering.
///
/// Walks edges in the specified direction up to `max_hops`, skipping edges
/// whose `confidence` is `Some(c)` with `c < min_confidence`.  Edges with
/// `confidence: None` are always traversed.
fn bfs_with_confidence(
    root_id: &str,
    max_hops: u32,
    min_confidence: Option<f64>,
    nodes: &HashMap<String, GraphNode>,
    edges: &[GraphEdge],
    direction: BfsDirection,
) -> Subgraph {
    let mut result_nodes = Vec::new();
    let mut seen_edges: HashSet<usize> = HashSet::new();
    let mut result_edges = Vec::new();
    let mut visited = HashSet::new();
    let mut frontier = vec![root_id.to_owned()];

    for _depth in 0..max_hops {
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
                // Confidence gate: skip edges below threshold.
                if let Some(threshold) = min_confidence {
                    if let Some(c) = edge.confidence {
                        if c < threshold {
                            continue;
                        }
                    }
                }

                let neighbor = match direction {
                    BfsDirection::Downstream => {
                        if edge.source_node_id == *nid {
                            &edge.target_node_id
                        } else {
                            continue;
                        }
                    }
                    BfsDirection::Upstream => {
                        if edge.target_node_id == *nid {
                            &edge.source_node_id
                        } else {
                            continue;
                        }
                    }
                };

                if seen_edges.insert(ei) {
                    result_edges.push(edge.clone());
                }
                next.push(neighbor.clone());
            }
        }
        frontier = next;
    }

    Subgraph {
        nodes: result_nodes,
        edges: result_edges,
    }
}

/// BFS shortest path between two nodes, following edges bidirectionally.
///
/// Returns `None` if no path exists within `max_depth`.
fn bfs_shortest_path(
    from_id: &str,
    to_id: &str,
    max_depth: u32,
    nodes: &HashMap<String, GraphNode>,
    edges: &[GraphEdge],
    edge_filter: Option<EdgeKind>,
) -> Option<Subgraph> {
    use std::collections::VecDeque;

    // BFS with parent tracking: parent[node_id] = (parent_node_id, edge_index)
    let mut parent: HashMap<String, (String, usize)> = HashMap::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();

    visited.insert(from_id.to_owned());
    queue.push_back((from_id.to_owned(), 0));

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        for (ei, edge) in edges.iter().enumerate() {
            if let Some(kind) = edge_filter {
                if edge.kind != kind {
                    continue;
                }
            }

            let neighbor = if edge.source_node_id == current {
                &edge.target_node_id
            } else if edge.target_node_id == current {
                &edge.source_node_id
            } else {
                continue;
            };

            if visited.contains(neighbor.as_str()) {
                continue;
            }

            visited.insert(neighbor.clone());
            parent.insert(neighbor.clone(), (current.clone(), ei));

            if neighbor == to_id {
                // Reconstruct path.
                let mut path_nodes = Vec::new();
                let mut path_edges = Vec::new();
                let mut cursor = to_id.to_owned();

                while let Some((prev, edge_idx)) = parent.get(&cursor) {
                    path_edges.push(edges[*edge_idx].clone());
                    if let Some(node) = nodes.get(&cursor) {
                        path_nodes.push(node.clone());
                    }
                    cursor = prev.clone();
                }
                // Add the source node.
                if let Some(node) = nodes.get(from_id) {
                    path_nodes.push(node.clone());
                }
                path_nodes.reverse();
                path_edges.reverse();

                return Some(Subgraph {
                    nodes: path_nodes,
                    edges: path_edges,
                });
            }

            queue.push_back((neighbor.clone(), depth + 1));
        }
    }

    None
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
                    agent_role_id: None,
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
                confidence: None,
            })
            .await
            .unwrap();
        store
            .add_edge(GraphEdge {
                source_node_id: "a".to_owned(),
                target_node_id: "c".to_owned(),
                kind: EdgeKind::UsedTool,
                created_at: 11,
                confidence: None,
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
            .neighbors(
                "a",
                Some(EdgeKind::Spawned),
                TraversalDirection::Downstream,
                10,
            )
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
        assert_eq!(node.project.as_ref().unwrap().project_id.as_str(), "docs");
    }

    // ── Gap 1: Edge query tests ──────────────────────────────────────────

    async fn build_chain_graph() -> Arc<InMemoryGraphStore> {
        // A --Triggered--> B --Spawned--> C --UsedTool--> D
        let store = Arc::new(InMemoryGraphStore::new());
        for (id, kind) in [
            ("a", NodeKind::Session),
            ("b", NodeKind::Run),
            ("c", NodeKind::Task),
            ("d", NodeKind::ToolInvocation),
        ] {
            store
                .add_node(GraphNode {
                    node_id: id.to_owned(),
                    kind,
                    project: None,
                    created_at: 1,
                })
                .await
                .unwrap();
        }
        for (src, tgt, kind) in [
            ("a", "b", EdgeKind::Triggered),
            ("b", "c", EdgeKind::Spawned),
            ("c", "d", EdgeKind::UsedTool),
        ] {
            store
                .add_edge(GraphEdge {
                    source_node_id: src.to_owned(),
                    target_node_id: tgt.to_owned(),
                    kind,
                    created_at: 10,
                    confidence: None,
                })
                .await
                .unwrap();
        }
        store
    }

    #[tokio::test]
    async fn find_edges_by_source_returns_outgoing() {
        let store = build_chain_graph().await;
        let edges = store.find_edges_by_source("a", None, 100).await.unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target_node_id, "b");
        assert_eq!(edges[0].kind, EdgeKind::Triggered);
    }

    #[tokio::test]
    async fn find_edges_by_source_with_filter() {
        let store = build_chain_graph().await;
        // "b" has one outgoing Spawned edge. Filtering by Triggered returns nothing.
        let edges = store
            .find_edges_by_source("b", Some(EdgeKind::Triggered), 100)
            .await
            .unwrap();
        assert!(edges.is_empty());

        let edges = store
            .find_edges_by_source("b", Some(EdgeKind::Spawned), 100)
            .await
            .unwrap();
        assert_eq!(edges.len(), 1);
    }

    #[tokio::test]
    async fn find_edges_by_target_returns_incoming() {
        let store = build_chain_graph().await;
        let edges = store.find_edges_by_target("d", None, 100).await.unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_node_id, "c");
    }

    #[tokio::test]
    async fn find_edges_by_target_respects_limit() {
        let store = Arc::new(InMemoryGraphStore::new());
        store
            .add_node(GraphNode {
                node_id: "target".into(),
                kind: NodeKind::Task,
                project: None,
                created_at: 1,
            })
            .await
            .unwrap();
        for i in 0..5 {
            store
                .add_node(GraphNode {
                    node_id: format!("src_{i}"),
                    kind: NodeKind::Run,
                    project: None,
                    created_at: 1,
                })
                .await
                .unwrap();
            store
                .add_edge(GraphEdge {
                    source_node_id: format!("src_{i}"),
                    target_node_id: "target".into(),
                    kind: EdgeKind::Spawned,
                    created_at: 10,
                    confidence: None,
                })
                .await
                .unwrap();
        }
        let edges = store.find_edges_by_target("target", None, 3).await.unwrap();
        assert_eq!(edges.len(), 3);
    }

    #[tokio::test]
    async fn shortest_path_direct_neighbor() {
        let store = build_chain_graph().await;
        let path = store.shortest_path("a", "b", None, 10).await.unwrap();
        let path = path.expect("path should exist");
        assert_eq!(path.nodes.len(), 2);
        assert_eq!(path.edges.len(), 1);
        assert_eq!(path.nodes[0].node_id, "a");
        assert_eq!(path.nodes[1].node_id, "b");
    }

    #[tokio::test]
    async fn shortest_path_multi_hop() {
        let store = build_chain_graph().await;
        // a -> b -> c -> d, shortest path from a to d is 3 hops
        let path = store.shortest_path("a", "d", None, 10).await.unwrap();
        let path = path.expect("path should exist");
        assert_eq!(path.nodes.len(), 4);
        assert_eq!(path.edges.len(), 3);
        let ids: Vec<&str> = path.nodes.iter().map(|n| n.node_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c", "d"]);
    }

    #[tokio::test]
    async fn shortest_path_same_node() {
        let store = build_chain_graph().await;
        let path = store.shortest_path("b", "b", None, 10).await.unwrap();
        let path = path.expect("same-node path");
        assert_eq!(path.nodes.len(), 1);
        assert!(path.edges.is_empty());
    }

    #[tokio::test]
    async fn shortest_path_no_path() {
        let store = Arc::new(InMemoryGraphStore::new());
        for id in ["x", "y"] {
            store
                .add_node(GraphNode {
                    node_id: id.to_owned(),
                    kind: NodeKind::Task,
                    project: None,
                    created_at: 1,
                })
                .await
                .unwrap();
        }
        // No edge between x and y.
        let path = store.shortest_path("x", "y", None, 10).await.unwrap();
        assert!(path.is_none());
    }

    #[tokio::test]
    async fn shortest_path_respects_max_depth() {
        let store = build_chain_graph().await;
        // a to d is 3 hops. Max depth 2 should not find it.
        let path = store.shortest_path("a", "d", None, 2).await.unwrap();
        assert!(path.is_none());
    }

    #[tokio::test]
    async fn shortest_path_with_edge_filter() {
        let store = build_chain_graph().await;
        // Only follow Triggered edges — can't reach d from a.
        let path = store
            .shortest_path("a", "d", Some(EdgeKind::Triggered), 10)
            .await
            .unwrap();
        assert!(path.is_none());
    }

    // ── MultiHop BFS tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn multi_hop_downstream_walks_chain() {
        // A --Triggered--> B --Spawned--> C --UsedTool--> D
        let store = build_chain_graph().await;

        let subgraph = store
            .query(GraphQuery::MultiHop {
                start_node_id: "a".to_owned(),
                max_hops: 4,
                min_confidence: None,
                direction: TraversalDirection::Downstream,
            })
            .await
            .unwrap();

        let ids: Vec<&str> = subgraph.nodes.iter().map(|n| n.node_id.as_str()).collect();
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
        assert!(ids.contains(&"d"));
        assert_eq!(subgraph.edges.len(), 3);
    }

    #[tokio::test]
    async fn multi_hop_respects_max_hops() {
        let store = build_chain_graph().await;

        // Only 2 hops from a: should reach a, b, c but not d.
        let subgraph = store
            .query(GraphQuery::MultiHop {
                start_node_id: "a".to_owned(),
                max_hops: 2,
                min_confidence: None,
                direction: TraversalDirection::Downstream,
            })
            .await
            .unwrap();

        let ids: Vec<&str> = subgraph.nodes.iter().map(|n| n.node_id.as_str()).collect();
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
        assert!(
            !ids.contains(&"d"),
            "d is 3 hops away, should not be reached"
        );
    }

    #[tokio::test]
    async fn multi_hop_upstream() {
        let store = build_chain_graph().await;

        let subgraph = store
            .query(GraphQuery::MultiHop {
                start_node_id: "d".to_owned(),
                max_hops: 10,
                min_confidence: None,
                direction: TraversalDirection::Upstream,
            })
            .await
            .unwrap();

        let ids: Vec<&str> = subgraph.nodes.iter().map(|n| n.node_id.as_str()).collect();
        assert!(ids.contains(&"d"));
        assert!(ids.contains(&"c"));
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"a"));
    }

    #[tokio::test]
    async fn multi_hop_filters_by_confidence() {
        let store = Arc::new(InMemoryGraphStore::new());

        // Build: X --high(0.9)--> Y --low(0.3)--> Z
        for id in ["x", "y", "z"] {
            store
                .add_node(GraphNode {
                    node_id: id.to_owned(),
                    kind: NodeKind::Document,
                    project: None,
                    created_at: 1,
                })
                .await
                .unwrap();
        }
        store
            .add_edge(GraphEdge {
                source_node_id: "x".to_owned(),
                target_node_id: "y".to_owned(),
                kind: EdgeKind::DerivedFrom,
                created_at: 10,
                confidence: Some(0.9),
            })
            .await
            .unwrap();
        store
            .add_edge(GraphEdge {
                source_node_id: "y".to_owned(),
                target_node_id: "z".to_owned(),
                kind: EdgeKind::DerivedFrom,
                created_at: 11,
                confidence: Some(0.3),
            })
            .await
            .unwrap();

        // Without confidence filter: reaches all three.
        let all = store
            .query(GraphQuery::MultiHop {
                start_node_id: "x".to_owned(),
                max_hops: 5,
                min_confidence: None,
                direction: TraversalDirection::Downstream,
            })
            .await
            .unwrap();
        assert_eq!(all.nodes.len(), 3);

        // With min_confidence=0.5: the 0.3 edge is pruned, so z is unreachable.
        let filtered = store
            .query(GraphQuery::MultiHop {
                start_node_id: "x".to_owned(),
                max_hops: 5,
                min_confidence: Some(0.5),
                direction: TraversalDirection::Downstream,
            })
            .await
            .unwrap();
        let ids: Vec<&str> = filtered.nodes.iter().map(|n| n.node_id.as_str()).collect();
        assert!(ids.contains(&"x"));
        assert!(ids.contains(&"y"));
        assert!(
            !ids.contains(&"z"),
            "z should be pruned by confidence filter"
        );
        assert_eq!(filtered.edges.len(), 1);
    }

    #[tokio::test]
    async fn multi_hop_handles_cycles() {
        let store = Arc::new(InMemoryGraphStore::new());

        // Build cycle: A -> B -> C -> A
        for id in ["a", "b", "c"] {
            store
                .add_node(GraphNode {
                    node_id: id.to_owned(),
                    kind: NodeKind::Task,
                    project: None,
                    created_at: 1,
                })
                .await
                .unwrap();
        }
        for (src, tgt) in [("a", "b"), ("b", "c"), ("c", "a")] {
            store
                .add_edge(GraphEdge {
                    source_node_id: src.to_owned(),
                    target_node_id: tgt.to_owned(),
                    kind: EdgeKind::Spawned,
                    created_at: 10,
                    confidence: None,
                })
                .await
                .unwrap();
        }

        // Should terminate despite cycle, visiting each node exactly once.
        let subgraph = store
            .query(GraphQuery::MultiHop {
                start_node_id: "a".to_owned(),
                max_hops: 10,
                min_confidence: None,
                direction: TraversalDirection::Downstream,
            })
            .await
            .unwrap();

        assert_eq!(
            subgraph.nodes.len(),
            3,
            "cycle detection should prevent infinite traversal"
        );
    }
}

#[async_trait::async_trait]
impl GraphQueryService for std::sync::Arc<InMemoryGraphStore> {
    async fn query(&self, query: GraphQuery) -> Result<Subgraph, GraphQueryError> {
        GraphQueryService::query(self.as_ref(), query).await
    }

    async fn neighbors(
        &self,
        node_id: &str,
        edge_filter: Option<crate::projections::EdgeKind>,
        direction: crate::queries::TraversalDirection,
        limit: usize,
    ) -> Result<Vec<(crate::projections::GraphEdge, crate::projections::GraphNode)>, GraphQueryError>
    {
        GraphQueryService::neighbors(self.as_ref(), node_id, edge_filter, direction, limit).await
    }

    async fn find_edges_by_source(
        &self,
        source_node_id: &str,
        edge_filter: Option<crate::projections::EdgeKind>,
        limit: usize,
    ) -> Result<Vec<crate::projections::GraphEdge>, GraphQueryError> {
        GraphQueryService::find_edges_by_source(self.as_ref(), source_node_id, edge_filter, limit)
            .await
    }

    async fn find_edges_by_target(
        &self,
        target_node_id: &str,
        edge_filter: Option<crate::projections::EdgeKind>,
        limit: usize,
    ) -> Result<Vec<crate::projections::GraphEdge>, GraphQueryError> {
        GraphQueryService::find_edges_by_target(self.as_ref(), target_node_id, edge_filter, limit)
            .await
    }

    async fn shortest_path(
        &self,
        from_node_id: &str,
        to_node_id: &str,
        edge_filter: Option<crate::projections::EdgeKind>,
        max_depth: u32,
    ) -> Result<Option<Subgraph>, GraphQueryError> {
        GraphQueryService::shortest_path(
            self.as_ref(),
            from_node_id,
            to_node_id,
            edge_filter,
            max_depth,
        )
        .await
    }
}
