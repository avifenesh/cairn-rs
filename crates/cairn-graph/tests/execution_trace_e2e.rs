//! RFC 004 — graph execution trace query end-to-end integration tests.
//!
//! Tests the execution trace subgraph query:
//!   1. Create a run node in the graph
//!   2. Add task nodes linked to the run via Spawned edges
//!   3. Add tool invocation nodes linked to tasks via UsedTool edges
//!   4. Query ExecutionTrace for the run — verify the subgraph contains
//!      all directly and transitively linked nodes
//!
//! Additional coverage:
//!   - nodes() snapshot via all_nodes() / all_edges() helpers
//!   - Traversal respects max_depth
//!   - Missing root node returns NodeNotFound
//!   - DependencyPath query follows Spawned/DependedOn edges
//!   - neighbors() returns immediate linked nodes

use std::sync::Arc;

use cairn_domain::ProjectKey;
use cairn_graph::in_memory::InMemoryGraphStore;
use cairn_graph::projections::{EdgeKind, GraphEdge, GraphNode, GraphProjection, NodeKind};
use cairn_graph::queries::{GraphQuery, GraphQueryService, TraversalDirection};

fn project() -> ProjectKey {
    ProjectKey::new("t_graph", "w_graph", "p_graph")
}

fn node(id: &str, kind: NodeKind) -> GraphNode {
    GraphNode {
        node_id: id.to_owned(),
        kind,
        project: Some(project()),
        created_at: 1_700_000_000_000,
    }
}

fn edge(src: &str, tgt: &str, kind: EdgeKind) -> GraphEdge {
    GraphEdge {
        source_node_id: src.to_owned(),
        target_node_id: tgt.to_owned(),
        kind,
        created_at: 1_700_000_000_001,
        confidence: None,
    }
}

// ── Tests 1–4: build graph and query execution trace ─────────────────────────

/// RFC 004 §4: ExecutionTrace starting from a run must return the run node
/// plus all task and tool-invocation nodes reachable via Spawned and UsedTool
/// edges within the configured max_depth.
#[tokio::test]
async fn execution_trace_returns_full_subgraph() {
    let graph = Arc::new(InMemoryGraphStore::new());

    // ── (1) Create a run node ─────────────────────────────────────────────
    graph
        .add_node(node("run_trace_1", NodeKind::Run))
        .await
        .unwrap();

    // ── (2) Add task nodes linked to the run via Spawned edges ────────────
    graph
        .add_node(node("task_trace_1", NodeKind::Task))
        .await
        .unwrap();
    graph
        .add_node(node("task_trace_2", NodeKind::Task))
        .await
        .unwrap();

    // Run → task via Spawned.
    graph
        .add_edge(edge("run_trace_1", "task_trace_1", EdgeKind::Spawned))
        .await
        .unwrap();
    graph
        .add_edge(edge("run_trace_1", "task_trace_2", EdgeKind::Spawned))
        .await
        .unwrap();

    // ── (3) Add tool invocation nodes linked to tasks via UsedTool edges ──
    graph
        .add_node(node("tool_inv_1", NodeKind::ToolInvocation))
        .await
        .unwrap();
    graph
        .add_node(node("tool_inv_2", NodeKind::ToolInvocation))
        .await
        .unwrap();

    // task1 → tool_inv_1, task2 → tool_inv_2 via UsedTool.
    graph
        .add_edge(edge("task_trace_1", "tool_inv_1", EdgeKind::UsedTool))
        .await
        .unwrap();
    graph
        .add_edge(edge("task_trace_2", "tool_inv_2", EdgeKind::UsedTool))
        .await
        .unwrap();

    // Unrelated node (must NOT appear in the trace).
    graph
        .add_node(node("unrelated_node", NodeKind::Document))
        .await
        .unwrap();

    // ── (4) Query ExecutionTrace — verify subgraph contains all linked nodes
    let result = graph
        .query(GraphQuery::ExecutionTrace {
            root_node_id: "run_trace_1".to_owned(),
            root_kind: NodeKind::Run,
            max_depth: 5,
        })
        .await
        .unwrap();

    // Collect returned node IDs.
    let node_ids: Vec<&str> = result.nodes.iter().map(|n| n.node_id.as_str()).collect();

    // Run itself must be present.
    assert!(
        node_ids.contains(&"run_trace_1"),
        "RFC 004: run node must be in the execution trace"
    );

    // Both tasks must be present.
    assert!(
        node_ids.contains(&"task_trace_1"),
        "RFC 004: task_trace_1 must be in the execution trace"
    );
    assert!(
        node_ids.contains(&"task_trace_2"),
        "RFC 004: task_trace_2 must be in the execution trace"
    );

    // Both tool invocations must be present.
    assert!(
        node_ids.contains(&"tool_inv_1"),
        "RFC 004: tool_inv_1 must be in the execution trace"
    );
    assert!(
        node_ids.contains(&"tool_inv_2"),
        "RFC 004: tool_inv_2 must be in the execution trace"
    );

    // Unrelated node must NOT be in the trace.
    assert!(
        !node_ids.contains(&"unrelated_node"),
        "RFC 004: unrelated_node must not appear in execution trace"
    );

    // Verify total node count (run + 2 tasks + 2 tool invocations = 5).
    assert_eq!(
        result.nodes.len(),
        5,
        "execution trace must contain exactly 5 nodes; got: {:?}",
        node_ids
    );

    // Verify the edges connecting them are also returned.
    assert!(
        !result.edges.is_empty(),
        "execution trace must include the edges connecting the nodes"
    );
    let has_spawned = result.edges.iter().any(|e| e.kind == EdgeKind::Spawned);
    let has_used_tool = result.edges.iter().any(|e| e.kind == EdgeKind::UsedTool);
    assert!(has_spawned, "Spawned edges must appear in the trace");
    assert!(has_used_tool, "UsedTool edges must appear in the trace");
}

// ── Node and edge inspection via helpers ─────────────────────────────────────

/// all_nodes() and all_edges() helpers must reflect every inserted node/edge.
#[tokio::test]
async fn all_nodes_and_edges_helpers_reflect_full_graph() {
    let graph = Arc::new(InMemoryGraphStore::new());

    graph.add_node(node("n1", NodeKind::Run)).await.unwrap();
    graph.add_node(node("n2", NodeKind::Task)).await.unwrap();
    graph
        .add_edge(edge("n1", "n2", EdgeKind::Spawned))
        .await
        .unwrap();

    let nodes = graph.all_nodes();
    let edges = graph.all_edges();

    assert_eq!(nodes.len(), 2);
    assert!(nodes.contains_key("n1"));
    assert!(nodes.contains_key("n2"));

    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].kind, EdgeKind::Spawned);
    assert_eq!(edges[0].source_node_id, "n1");
    assert_eq!(edges[0].target_node_id, "n2");
}

// ── node_exists() reflects inserts ───────────────────────────────────────────

#[tokio::test]
async fn node_exists_returns_correct_presence() {
    let graph = Arc::new(InMemoryGraphStore::new());

    assert!(!graph.node_exists("no_such").await.unwrap());
    graph
        .add_node(node("present", NodeKind::Task))
        .await
        .unwrap();
    assert!(graph.node_exists("present").await.unwrap());
    assert!(!graph.node_exists("still_missing").await.unwrap());
}

// ── max_depth limits traversal ───────────────────────────────────────────────

/// max_depth controls how many BFS iterations run.
/// Iteration 0 visits the root; iteration 1 visits root's neighbors; etc.
/// So max_depth=1 → root only; max_depth=2 → root + direct neighbors;
/// max_depth=3 → root + neighbors + their neighbors.
#[tokio::test]
async fn execution_trace_respects_max_depth() {
    let graph = Arc::new(InMemoryGraphStore::new());

    // run → task → tool (run is depth-0, task depth-1, tool depth-2)
    graph
        .add_node(node("root_run", NodeKind::Run))
        .await
        .unwrap();
    graph
        .add_node(node("mid_task", NodeKind::Task))
        .await
        .unwrap();
    graph
        .add_node(node("deep_tool", NodeKind::ToolInvocation))
        .await
        .unwrap();

    graph
        .add_edge(edge("root_run", "mid_task", EdgeKind::Spawned))
        .await
        .unwrap();
    graph
        .add_edge(edge("mid_task", "deep_tool", EdgeKind::UsedTool))
        .await
        .unwrap();

    // max_depth=1 → only root (one BFS iteration processes root, pushes neighbors
    // into next frontier, but the loop ends before processing them).
    let depth1 = graph
        .query(GraphQuery::ExecutionTrace {
            root_node_id: "root_run".to_owned(),
            root_kind: NodeKind::Run,
            max_depth: 1,
        })
        .await
        .unwrap();

    let d1_ids: Vec<&str> = depth1.nodes.iter().map(|n| n.node_id.as_str()).collect();
    assert!(
        d1_ids.contains(&"root_run"),
        "root must be present at max_depth=1"
    );
    assert!(
        !d1_ids.contains(&"deep_tool"),
        "deep_tool (depth-2) must not appear at max_depth=1"
    );

    // max_depth=2 → root + direct neighbors (mid_task), but not deep_tool.
    let depth2 = graph
        .query(GraphQuery::ExecutionTrace {
            root_node_id: "root_run".to_owned(),
            root_kind: NodeKind::Run,
            max_depth: 2,
        })
        .await
        .unwrap();
    let d2_ids: Vec<&str> = depth2.nodes.iter().map(|n| n.node_id.as_str()).collect();
    assert!(d2_ids.contains(&"root_run"), "root at depth2");
    assert!(d2_ids.contains(&"mid_task"), "direct neighbor at depth2");
    assert!(
        !d2_ids.contains(&"deep_tool"),
        "deep_tool must not appear at max_depth=2"
    );

    // max_depth=3 → all three nodes reachable.
    let depth3 = graph
        .query(GraphQuery::ExecutionTrace {
            root_node_id: "root_run".to_owned(),
            root_kind: NodeKind::Run,
            max_depth: 3,
        })
        .await
        .unwrap();
    let d3_ids: Vec<&str> = depth3.nodes.iter().map(|n| n.node_id.as_str()).collect();
    assert!(
        d3_ids.contains(&"deep_tool"),
        "deep_tool must appear at max_depth=3"
    );
}

// ── Missing root node returns an empty subgraph ──────────────────────────────

/// RFC 004: querying a root node that does not exist in the graph must
/// return an empty subgraph (no nodes, no edges) — the BFS simply finds
/// nothing reachable from the missing root.
#[tokio::test]
async fn execution_trace_missing_root_returns_empty_subgraph() {
    let graph = Arc::new(InMemoryGraphStore::new());

    let result = graph
        .query(GraphQuery::ExecutionTrace {
            root_node_id: "ghost_run".to_owned(),
            root_kind: NodeKind::Run,
            max_depth: 5,
        })
        .await
        .unwrap(); // No error — just an empty result.

    assert!(
        result.nodes.is_empty(),
        "RFC 004: trace from a non-existent root must return no nodes; got: {:?}",
        result.nodes.iter().map(|n| &n.node_id).collect::<Vec<_>>()
    );
    assert!(
        result.edges.is_empty(),
        "RFC 004: trace from a non-existent root must return no edges"
    );
}

// ── DependencyPath follows Spawned/DependedOn edges ──────────────────────────

/// RFC 004: DependencyPath must follow Spawned and DependedOn edges
/// in the requested direction.
#[tokio::test]
async fn dependency_path_follows_spawned_edges_downstream() {
    let graph = Arc::new(InMemoryGraphStore::new());

    // parent_run → child_task_a → child_task_b (chained spawns)
    graph
        .add_node(node("parent_run", NodeKind::Run))
        .await
        .unwrap();
    graph
        .add_node(node("child_task_a", NodeKind::Task))
        .await
        .unwrap();
    graph
        .add_node(node("child_task_b", NodeKind::Task))
        .await
        .unwrap();

    graph
        .add_edge(edge("parent_run", "child_task_a", EdgeKind::Spawned))
        .await
        .unwrap();
    graph
        .add_edge(edge("child_task_a", "child_task_b", EdgeKind::Spawned))
        .await
        .unwrap();

    let result = graph
        .query(GraphQuery::DependencyPath {
            node_id: "parent_run".to_owned(),
            direction: TraversalDirection::Downstream,
            max_depth: 5,
        })
        .await
        .unwrap();

    let ids: Vec<&str> = result.nodes.iter().map(|n| n.node_id.as_str()).collect();
    assert!(ids.contains(&"parent_run"), "root must be present");
    assert!(
        ids.contains(&"child_task_a"),
        "direct child must be present"
    );
    assert!(
        ids.contains(&"child_task_b"),
        "transitive child must be present"
    );
}

// ── neighbors() returns immediate neighbors filtered by edge kind ─────────────

#[tokio::test]
async fn neighbors_returns_immediate_edges_filtered_by_kind() {
    let graph = Arc::new(InMemoryGraphStore::new());

    // hub → three nodes via different edge kinds
    graph.add_node(node("hub", NodeKind::Run)).await.unwrap();
    graph.add_node(node("task1", NodeKind::Task)).await.unwrap();
    graph
        .add_node(node("tool1", NodeKind::ToolInvocation))
        .await
        .unwrap();
    graph
        .add_node(node("memo1", NodeKind::Memory))
        .await
        .unwrap();

    graph
        .add_edge(edge("hub", "task1", EdgeKind::Spawned))
        .await
        .unwrap();
    graph
        .add_edge(edge("hub", "tool1", EdgeKind::UsedTool))
        .await
        .unwrap();
    graph
        .add_edge(edge("hub", "memo1", EdgeKind::ReadFrom))
        .await
        .unwrap();

    // All downstream neighbors.
    let all_neighbors = graph
        .neighbors("hub", None, TraversalDirection::Downstream, 10)
        .await
        .unwrap();
    assert_eq!(
        all_neighbors.len(),
        3,
        "hub must have 3 downstream neighbors"
    );

    // Only Spawned neighbors.
    let spawned_only = graph
        .neighbors(
            "hub",
            Some(EdgeKind::Spawned),
            TraversalDirection::Downstream,
            10,
        )
        .await
        .unwrap();
    assert_eq!(spawned_only.len(), 1, "only 1 Spawned neighbor");
    assert_eq!(spawned_only[0].1.node_id, "task1");

    // Only UsedTool neighbors.
    let tool_only = graph
        .neighbors(
            "hub",
            Some(EdgeKind::UsedTool),
            TraversalDirection::Downstream,
            10,
        )
        .await
        .unwrap();
    assert_eq!(tool_only.len(), 1);
    assert_eq!(tool_only[0].1.node_id, "tool1");
}
