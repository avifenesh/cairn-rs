use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::projections::{EdgeKind, GraphEdge, GraphNode, NodeKind};

/// Query families optimized for v1 (RFC 004).
///
/// V1 does not require arbitrary graph analytics. These are the
/// product-shaped query families that the graph layer must support.
#[derive(Clone, Debug)]
pub enum GraphQuery {
    /// Execution trace for a session, run, or task.
    ExecutionTrace {
        root_node_id: String,
        root_kind: NodeKind,
        max_depth: u32,
    },
    /// Subagent/task dependency path and resume lineage.
    DependencyPath {
        node_id: String,
        direction: TraversalDirection,
        max_depth: u32,
    },
    /// Prompt provenance for a runtime outcome.
    PromptProvenance { outcome_node_id: String },
    /// Retrieval provenance: answer -> chunk -> document -> source.
    RetrievalProvenance { answer_node_id: String },
    /// Tool and policy involvement for a runtime decision.
    DecisionInvolvement { decision_node_id: String },
    /// Eval-to-asset lineage for prompt releases and provider routes.
    EvalLineage { eval_run_node_id: String },
    /// Generic multi-hop BFS traversal from a start node.
    ///
    /// Walks edges up to `max_hops` with cycle detection. When
    /// `min_confidence` is set, edges whose `confidence` field is
    /// `Some(c)` with `c < min_confidence` are skipped (edges with
    /// `confidence: None` are always traversed).
    MultiHop {
        start_node_id: String,
        max_hops: u32,
        min_confidence: Option<f64>,
        direction: TraversalDirection,
    },
}

/// Traversal direction for dependency queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraversalDirection {
    Upstream,
    Downstream,
}

/// A subgraph result from a graph query.
#[derive(Clone, Debug)]
pub struct Subgraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Product-shaped graph query service (RFC 004).
///
/// V1 exposes graph capabilities through named query families aligned
/// to product workflows, not a fully general traversal API.
#[async_trait]
pub trait GraphQueryService: Send + Sync {
    /// Execute a product-shaped graph query.
    async fn query(&self, query: GraphQuery) -> Result<Subgraph, GraphQueryError>;

    /// Get the immediate neighbors of a node, optionally filtered by edge kind.
    async fn neighbors(
        &self,
        node_id: &str,
        edge_filter: Option<EdgeKind>,
        direction: TraversalDirection,
        limit: usize,
    ) -> Result<Vec<(GraphEdge, GraphNode)>, GraphQueryError>;

    /// Find all edges originating from a given source node.
    async fn find_edges_by_source(
        &self,
        source_node_id: &str,
        edge_filter: Option<EdgeKind>,
        limit: usize,
    ) -> Result<Vec<GraphEdge>, GraphQueryError>;

    /// Find all edges targeting a given node.
    async fn find_edges_by_target(
        &self,
        target_node_id: &str,
        edge_filter: Option<EdgeKind>,
        limit: usize,
    ) -> Result<Vec<GraphEdge>, GraphQueryError>;

    /// Find the shortest path between two nodes using BFS.
    ///
    /// Returns `None` if no path exists within `max_depth` hops.
    /// The path includes both endpoint nodes and all edges traversed.
    async fn shortest_path(
        &self,
        from_node_id: &str,
        to_node_id: &str,
        edge_filter: Option<EdgeKind>,
        max_depth: u32,
    ) -> Result<Option<Subgraph>, GraphQueryError>;
}

/// Graph query errors.
#[derive(Debug)]
pub enum GraphQueryError {
    NodeNotFound(String),
    DepthExceeded { max: u32 },
    StorageError(String),
    Internal(String),
}

impl std::fmt::Display for GraphQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphQueryError::NodeNotFound(id) => write!(f, "node not found: {id}"),
            GraphQueryError::DepthExceeded { max } => {
                write!(f, "traversal depth exceeded max {max}")
            }
            GraphQueryError::StorageError(msg) => write!(f, "storage error: {msg}"),
            GraphQueryError::Internal(msg) => write!(f, "internal graph query error: {msg}"),
        }
    }
}

impl std::error::Error for GraphQueryError {}

#[cfg(test)]
mod tests {
    use super::TraversalDirection;

    #[test]
    fn traversal_directions_are_distinct() {
        assert_ne!(TraversalDirection::Upstream, TraversalDirection::Downstream);
    }
}
