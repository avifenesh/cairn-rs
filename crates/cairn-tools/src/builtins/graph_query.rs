//! graph_query — multi-hop graph traversal via `GraphQueryService`.
//!
//! Gives the agent self-awareness of the knowledge and execution graph
//! around a node, enabling it to discover related documents, sibling runs,
//! dependent tasks, and retrieval provenance.
//!
//! ## Parameters
//! ```json
//! {
//!   "node_id":        "run_abc123",
//!   "direction":      "downstream",   // "upstream" | "downstream"
//!   "max_hops":       3,              // default 2
//!   "min_confidence": 0.5             // optional; prune low-confidence edges
//! }
//! ```
//!
//! ## Output
//! ```json
//! {
//!   "nodes": [{ "node_id": "...", "kind": "run", "created_at": 0 }],
//!   "edges": [{ "source": "...", "target": "...", "kind": "spawned" }],
//!   "node_count": 4,
//!   "edge_count": 3
//! }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey};
use cairn_graph::queries::{GraphQuery, GraphQueryService, TraversalDirection};
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

/// Multi-hop graph traversal tool.
pub struct GraphQueryTool {
    graph: Arc<dyn GraphQueryService>,
}

impl GraphQueryTool {
    pub fn new(graph: Arc<dyn GraphQueryService>) -> Self {
        Self { graph }
    }
}

#[async_trait]
impl ToolHandler for GraphQueryTool {
    fn name(&self) -> &str {
        "graph_query"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }

    fn description(&self) -> &str {
        "Traverse the knowledge/execution graph from a node. Returns related nodes and edges."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["node_id"],
            "properties": {
                "node_id": {
                    "type": "string",
                    "description": "ID of the starting node (run_id, task_id, doc_id, etc.)."
                },
                "direction": {
                    "type": "string",
                    "enum": ["upstream", "downstream"],
                    "default": "downstream",
                    "description": "Traversal direction relative to the start node."
                },
                "max_hops": {
                    "type": "integer",
                    "default": 2,
                    "description": "Maximum number of hops from the start node."
                },
                "min_confidence": {
                    "type": "number",
                    "description": "Skip edges with confidence below this threshold (0.0–1.0)."
                }
            }
        })
    }

    // Read-only graph access — no approval required.
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let node_id = args
            .get("node_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "node_id".into(),
                message: "required".into(),
            })?
            .to_owned();

        if node_id.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "node_id".into(),
                message: "must not be empty".into(),
            });
        }

        let direction = match args
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("downstream")
        {
            "upstream" => TraversalDirection::Upstream,
            "downstream" => TraversalDirection::Downstream,
            other => {
                return Err(ToolError::InvalidArgs {
                    field: "direction".into(),
                    message: format!("must be 'upstream' or 'downstream', got '{other}'"),
                })
            }
        };

        let max_hops = args
            .get("max_hops")
            .and_then(|v| v.as_u64())
            .unwrap_or(2)
            .min(10) as u32; // cap at 10 to prevent runaway traversals

        let min_confidence = args.get("min_confidence").and_then(|v| v.as_f64());

        let query = GraphQuery::MultiHop {
            start_node_id: node_id,
            max_hops,
            min_confidence,
            direction,
        };

        let subgraph = self
            .graph
            .query(query)
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        let nodes: Vec<Value> = subgraph
            .nodes
            .iter()
            .map(|n| {
                serde_json::json!({
                    "node_id":    n.node_id,
                    "kind":       format!("{:?}", n.kind).to_lowercase(),
                    "created_at": n.created_at,
                })
            })
            .collect();

        let edges: Vec<Value> = subgraph
            .edges
            .iter()
            .map(|e| {
                let mut obj = serde_json::json!({
                    "source": e.source_node_id,
                    "target": e.target_node_id,
                    "kind":   format!("{:?}", e.kind).to_lowercase(),
                });
                if let Some(c) = e.confidence {
                    obj["confidence"] = serde_json::json!(c);
                }
                obj
            })
            .collect();

        let node_count = nodes.len();
        let edge_count = edges.len();

        Ok(ToolResult::ok(serde_json::json!({
            "nodes":      nodes,
            "edges":      edges,
            "node_count": node_count,
            "edge_count": edge_count,
        })))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cairn_graph::{
        projections::{EdgeKind, GraphEdge, GraphNode, NodeKind},
        queries::{GraphQuery, GraphQueryError, GraphQueryService, Subgraph, TraversalDirection},
    };
    use std::sync::Arc;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    // ── Minimal stub ──────────────────────────────────────────────────────────

    struct StubGraph {
        nodes: Vec<GraphNode>,
        edges: Vec<GraphEdge>,
    }

    #[async_trait]
    impl GraphQueryService for StubGraph {
        async fn query(&self, _q: GraphQuery) -> Result<Subgraph, GraphQueryError> {
            Ok(Subgraph {
                nodes: self.nodes.clone(),
                edges: self.edges.clone(),
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

    fn make_tool(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> GraphQueryTool {
        GraphQueryTool::new(Arc::new(StubGraph { nodes, edges }))
    }

    fn node(id: &str) -> GraphNode {
        GraphNode {
            node_id: id.to_owned(),
            kind: NodeKind::Run,
            project: None,
            created_at: 0,
        }
    }

    fn edge(src: &str, tgt: &str) -> GraphEdge {
        GraphEdge {
            source_node_id: src.to_owned(),
            target_node_id: tgt.to_owned(),
            kind: EdgeKind::Spawned,
            created_at: 0,
            confidence: None,
        }
    }

    // ── Metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn name_tier_class() {
        let t = make_tool(vec![], vec![]);
        assert_eq!(t.name(), "graph_query");
        assert_eq!(t.tier(), ToolTier::Registered);
        assert_eq!(t.execution_class(), ExecutionClass::SupervisedProcess);
    }

    #[test]
    fn schema_requires_node_id() {
        let req = make_tool(vec![], vec![]).parameters_schema()["required"]
            .as_array()
            .unwrap()
            .clone();
        assert!(req.iter().any(|v| v.as_str() == Some("node_id")));
    }

    // ── Validation ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn missing_node_id_is_invalid() {
        let err = make_tool(vec![], vec![])
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn empty_node_id_is_invalid() {
        let err = make_tool(vec![], vec![])
            .execute(&project(), serde_json::json!({"node_id": "  "}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn bad_direction_is_invalid() {
        let err = make_tool(vec![], vec![])
            .execute(
                &project(),
                serde_json::json!({"node_id": "n1", "direction": "sideways"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { field, .. } if field == "direction"));
    }

    // ── Happy paths ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn returns_nodes_and_edges() {
        let t = make_tool(
            vec![node("run_1"), node("run_2")],
            vec![edge("run_1", "run_2")],
        );
        let result = t
            .execute(&project(), serde_json::json!({"node_id": "run_1"}))
            .await
            .unwrap();

        assert_eq!(result.output["node_count"], 2);
        assert_eq!(result.output["edge_count"], 1);

        let nodes = result.output["nodes"].as_array().unwrap();
        assert!(nodes.iter().any(|n| n["node_id"] == "run_1"));
        assert!(nodes.iter().any(|n| n["node_id"] == "run_2"));

        let edges = result.output["edges"].as_array().unwrap();
        assert_eq!(edges[0]["source"], "run_1");
        assert_eq!(edges[0]["target"], "run_2");
    }

    #[tokio::test]
    async fn empty_graph_returns_zeros() {
        let result = make_tool(vec![], vec![])
            .execute(
                &project(),
                serde_json::json!({"node_id": "run_x", "max_hops": 5}),
            )
            .await
            .unwrap();
        assert_eq!(result.output["node_count"], 0);
        assert_eq!(result.output["edge_count"], 0);
    }

    #[tokio::test]
    async fn confidence_on_edge_is_included() {
        let mut e = edge("a", "b");
        e.confidence = Some(0.75);
        let result = make_tool(vec![node("a"), node("b")], vec![e])
            .execute(&project(), serde_json::json!({"node_id": "a"}))
            .await
            .unwrap();
        let edges = result.output["edges"].as_array().unwrap();
        assert!((edges[0]["confidence"].as_f64().unwrap() - 0.75).abs() < 1e-9);
    }

    #[tokio::test]
    async fn upstream_direction_accepted() {
        let result = make_tool(vec![], vec![])
            .execute(
                &project(),
                serde_json::json!({"node_id": "run_1", "direction": "upstream"}),
            )
            .await
            .unwrap();
        assert_eq!(result.output["node_count"], 0); // stub returns empty either way
    }
}
