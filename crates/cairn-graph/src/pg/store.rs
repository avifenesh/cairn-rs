use async_trait::async_trait;
use sqlx::PgPool;

use crate::projections::{
    EdgeKind, GraphEdge, GraphNode, GraphProjection, GraphProjectionError, NodeKind,
};
use crate::queries::{
    GraphQuery, GraphQueryError, GraphQueryService, Subgraph, TraversalDirection,
};

/// Postgres-backed graph store implementing projection and query traits.
///
/// Stores nodes and edges in the shared cairn-store schema
/// (V012/V013 migrations).
pub struct PgGraphStore {
    pool: PgPool,
}

impl PgGraphStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GraphProjection for PgGraphStore {
    async fn add_node(&self, node: GraphNode) -> Result<(), GraphProjectionError> {
        let kind_str = node_kind_str(node.kind);

        sqlx::query(
            "INSERT INTO graph_nodes (node_id, kind, created_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (node_id) DO NOTHING",
        )
        .bind(&node.node_id)
        .bind(kind_str)
        .bind(node.created_at as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| GraphProjectionError::StorageError(e.to_string()))?;

        Ok(())
    }

    async fn add_edge(&self, edge: GraphEdge) -> Result<(), GraphProjectionError> {
        let kind_str = edge_kind_str(edge.kind);

        sqlx::query(
            "INSERT INTO graph_edges (source_node_id, target_node_id, kind, created_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (source_node_id, target_node_id, kind) DO NOTHING",
        )
        .bind(&edge.source_node_id)
        .bind(&edge.target_node_id)
        .bind(kind_str)
        .bind(edge.created_at as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| GraphProjectionError::StorageError(e.to_string()))?;

        Ok(())
    }

    async fn node_exists(&self, node_id: &str) -> Result<bool, GraphProjectionError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM graph_nodes WHERE node_id = $1")
            .bind(node_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| GraphProjectionError::StorageError(e.to_string()))?;

        Ok(row.is_some())
    }
}

#[async_trait]
impl GraphQueryService for PgGraphStore {
    async fn query(&self, query: GraphQuery) -> Result<Subgraph, GraphQueryError> {
        match query {
            GraphQuery::ExecutionTrace {
                root_node_id,
                max_depth,
                ..
            } => self.traverse_downstream(&root_node_id, max_depth).await,
            GraphQuery::DependencyPath {
                node_id,
                direction,
                max_depth,
            } => match direction {
                TraversalDirection::Upstream => self.traverse_upstream(&node_id, max_depth).await,
                TraversalDirection::Downstream => {
                    self.traverse_downstream(&node_id, max_depth).await
                }
            },
            GraphQuery::PromptProvenance { outcome_node_id } => {
                self.traverse_upstream(&outcome_node_id, 10).await
            }
            GraphQuery::RetrievalProvenance { answer_node_id } => {
                self.traverse_upstream(&answer_node_id, 5).await
            }
            GraphQuery::DecisionInvolvement { decision_node_id } => {
                self.traverse_upstream(&decision_node_id, 5).await
            }
            GraphQuery::EvalLineage { eval_run_node_id } => {
                self.traverse_upstream(&eval_run_node_id, 10).await
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
        let (edge_col, join_col) = match direction {
            TraversalDirection::Downstream => ("source_node_id", "target_node_id"),
            TraversalDirection::Upstream => ("target_node_id", "source_node_id"),
        };

        let (edges, nodes) = if let Some(kind) = edge_filter {
            let kind_str = edge_kind_str(kind);
            let sql = format!(
                "SELECT e.source_node_id, e.target_node_id, e.kind, e.created_at,
                        n.node_id, n.kind AS node_kind, n.created_at AS node_created_at
                 FROM graph_edges e
                 JOIN graph_nodes n ON n.node_id = e.{join_col}
                 WHERE e.{edge_col} = $1 AND e.kind = $2
                 LIMIT $3"
            );
            fetch_neighbor_rows(&self.pool, &sql, node_id, Some(kind_str), limit).await?
        } else {
            let sql = format!(
                "SELECT e.source_node_id, e.target_node_id, e.kind, e.created_at,
                        n.node_id, n.kind AS node_kind, n.created_at AS node_created_at
                 FROM graph_edges e
                 JOIN graph_nodes n ON n.node_id = e.{join_col}
                 WHERE e.{edge_col} = $1
                 LIMIT $2"
            );
            fetch_neighbor_rows(&self.pool, &sql, node_id, None, limit).await?
        };

        Ok(edges.into_iter().zip(nodes).collect())
    }
}

impl PgGraphStore {
    /// BFS traversal downstream from a root node.
    async fn traverse_downstream(
        &self,
        root_id: &str,
        max_depth: u32,
    ) -> Result<Subgraph, GraphQueryError> {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut frontier = vec![root_id.to_owned()];
        let mut visited = std::collections::HashSet::new();

        for _depth in 0..max_depth {
            if frontier.is_empty() {
                break;
            }

            let mut next_frontier = Vec::new();

            for node_id in &frontier {
                if !visited.insert(node_id.clone()) {
                    continue;
                }

                // Fetch the node itself.
                if let Some(node) = fetch_node(&self.pool, node_id).await? {
                    nodes.push(node);
                }

                // Fetch outgoing edges.
                let out_edges = sqlx::query_as::<_, EdgeRow>(
                    "SELECT source_node_id, target_node_id, kind, created_at
                     FROM graph_edges WHERE source_node_id = $1",
                )
                .bind(node_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| GraphQueryError::StorageError(e.to_string()))?;

                for row in out_edges {
                    next_frontier.push(row.target_node_id.clone());
                    edges.push(row.into_graph_edge());
                }
            }

            frontier = next_frontier;
        }

        Ok(Subgraph { nodes, edges })
    }

    /// BFS traversal upstream from a leaf node.
    async fn traverse_upstream(
        &self,
        leaf_id: &str,
        max_depth: u32,
    ) -> Result<Subgraph, GraphQueryError> {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut frontier = vec![leaf_id.to_owned()];
        let mut visited = std::collections::HashSet::new();

        for _depth in 0..max_depth {
            if frontier.is_empty() {
                break;
            }

            let mut next_frontier = Vec::new();

            for node_id in &frontier {
                if !visited.insert(node_id.clone()) {
                    continue;
                }

                if let Some(node) = fetch_node(&self.pool, node_id).await? {
                    nodes.push(node);
                }

                let in_edges = sqlx::query_as::<_, EdgeRow>(
                    "SELECT source_node_id, target_node_id, kind, created_at
                     FROM graph_edges WHERE target_node_id = $1",
                )
                .bind(node_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| GraphQueryError::StorageError(e.to_string()))?;

                for row in in_edges {
                    next_frontier.push(row.source_node_id.clone());
                    edges.push(row.into_graph_edge());
                }
            }

            frontier = next_frontier;
        }

        Ok(Subgraph { nodes, edges })
    }
}

// --- Row types and helpers ---

#[derive(sqlx::FromRow)]
struct NodeRow {
    node_id: String,
    kind: String,
    created_at: i64,
}

impl NodeRow {
    fn into_graph_node(self) -> GraphNode {
        GraphNode {
            node_id: self.node_id,
            kind: parse_node_kind(&self.kind).unwrap_or(NodeKind::Session),
            project: None,
            created_at: self.created_at as u64,
        }
    }
}

#[derive(sqlx::FromRow)]
struct EdgeRow {
    source_node_id: String,
    target_node_id: String,
    kind: String,
    created_at: i64,
}

impl EdgeRow {
    fn into_graph_edge(self) -> GraphEdge {
        GraphEdge {
            source_node_id: self.source_node_id,
            target_node_id: self.target_node_id,
            kind: parse_edge_kind(&self.kind).unwrap_or(EdgeKind::Triggered),
            created_at: self.created_at as u64,
            confidence: None,
        }
    }
}

async fn fetch_node(pool: &PgPool, node_id: &str) -> Result<Option<GraphNode>, GraphQueryError> {
    let row: Option<NodeRow> =
        sqlx::query_as("SELECT node_id, kind, created_at FROM graph_nodes WHERE node_id = $1")
            .bind(node_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| GraphQueryError::StorageError(e.to_string()))?;

    Ok(row.map(|r| r.into_graph_node()))
}

async fn fetch_neighbor_rows(
    pool: &PgPool,
    sql: &str,
    node_id: &str,
    kind_filter: Option<&str>,
    limit: usize,
) -> Result<(Vec<GraphEdge>, Vec<GraphNode>), GraphQueryError> {
    #[derive(sqlx::FromRow)]
    struct NeighborRow {
        source_node_id: String,
        target_node_id: String,
        kind: String,
        created_at: i64,
        node_id: String,
        node_kind: String,
        node_created_at: i64,
    }

    let rows: Vec<NeighborRow> = if let Some(k) = kind_filter {
        sqlx::query_as(sql)
            .bind(node_id)
            .bind(k)
            .bind(limit as i64)
            .fetch_all(pool)
            .await
    } else {
        sqlx::query_as(sql)
            .bind(node_id)
            .bind(limit as i64)
            .fetch_all(pool)
            .await
    }
    .map_err(|e| GraphQueryError::StorageError(e.to_string()))?;

    let mut edges = Vec::with_capacity(rows.len());
    let mut nodes = Vec::with_capacity(rows.len());

    for r in rows {
        edges.push(GraphEdge {
            source_node_id: r.source_node_id,
            target_node_id: r.target_node_id,
            kind: parse_edge_kind(&r.kind).unwrap_or(EdgeKind::Triggered),
            created_at: r.created_at as u64,
            confidence: None,
        });
        nodes.push(GraphNode {
            node_id: r.node_id,
            kind: parse_node_kind(&r.node_kind).unwrap_or(NodeKind::Session),
            project: None,
            created_at: r.node_created_at as u64,
        });
    }

    Ok((edges, nodes))
}

fn node_kind_str(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Session => "session",
        NodeKind::Run => "run",
        NodeKind::Task => "task",
        NodeKind::Approval => "approval",
        NodeKind::Checkpoint => "checkpoint",
        NodeKind::MailboxMessage => "mailbox_message",
        NodeKind::ToolInvocation => "tool_invocation",
        NodeKind::Memory => "memory",
        NodeKind::Document => "document",
        NodeKind::Chunk => "chunk",
        NodeKind::Source => "source",
        NodeKind::PromptAsset => "prompt_asset",
        NodeKind::PromptVersion => "prompt_version",
        NodeKind::PromptRelease => "prompt_release",
        NodeKind::EvalRun => "eval_run",
        NodeKind::Skill => "skill",
        NodeKind::ChannelTarget => "channel_target",
        NodeKind::Signal => "signal",
        NodeKind::IngestJob => "ingest_job",
    }
}

fn parse_node_kind(s: &str) -> Option<NodeKind> {
    match s {
        "session" => Some(NodeKind::Session),
        "run" => Some(NodeKind::Run),
        "task" => Some(NodeKind::Task),
        "approval" => Some(NodeKind::Approval),
        "checkpoint" => Some(NodeKind::Checkpoint),
        "mailbox_message" => Some(NodeKind::MailboxMessage),
        "tool_invocation" => Some(NodeKind::ToolInvocation),
        "memory" => Some(NodeKind::Memory),
        "document" => Some(NodeKind::Document),
        "chunk" => Some(NodeKind::Chunk),
        "source" => Some(NodeKind::Source),
        "prompt_asset" => Some(NodeKind::PromptAsset),
        "prompt_version" => Some(NodeKind::PromptVersion),
        "prompt_release" => Some(NodeKind::PromptRelease),
        "eval_run" => Some(NodeKind::EvalRun),
        "skill" => Some(NodeKind::Skill),
        "channel_target" => Some(NodeKind::ChannelTarget),
        "signal" => Some(NodeKind::Signal),
        "ingest_job" => Some(NodeKind::IngestJob),
        _ => None,
    }
}

fn edge_kind_str(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Triggered => "triggered",
        EdgeKind::Spawned => "spawned",
        EdgeKind::DependedOn => "depended_on",
        EdgeKind::ApprovedBy => "approved_by",
        EdgeKind::ResumedFrom => "resumed_from",
        EdgeKind::SentTo => "sent_to",
        EdgeKind::ReadFrom => "read_from",
        EdgeKind::Cited => "cited",
        EdgeKind::DerivedFrom => "derived_from",
        EdgeKind::EmbeddedAs => "embedded_as",
        EdgeKind::EvaluatedBy => "evaluated_by",
        EdgeKind::ReleasedAs => "released_as",
        EdgeKind::RolledBackTo => "rolled_back_to",
        EdgeKind::RoutedTo => "routed_to",
        EdgeKind::UsedPrompt => "used_prompt",
        EdgeKind::UsedTool => "used_tool",
    }
}

fn parse_edge_kind(s: &str) -> Option<EdgeKind> {
    match s {
        "triggered" => Some(EdgeKind::Triggered),
        "spawned" => Some(EdgeKind::Spawned),
        "depended_on" => Some(EdgeKind::DependedOn),
        "approved_by" => Some(EdgeKind::ApprovedBy),
        "resumed_from" => Some(EdgeKind::ResumedFrom),
        "sent_to" => Some(EdgeKind::SentTo),
        "read_from" => Some(EdgeKind::ReadFrom),
        "cited" => Some(EdgeKind::Cited),
        "derived_from" => Some(EdgeKind::DerivedFrom),
        "embedded_as" => Some(EdgeKind::EmbeddedAs),
        "evaluated_by" => Some(EdgeKind::EvaluatedBy),
        "released_as" => Some(EdgeKind::ReleasedAs),
        "rolled_back_to" => Some(EdgeKind::RolledBackTo),
        "routed_to" => Some(EdgeKind::RoutedTo),
        "used_prompt" => Some(EdgeKind::UsedPrompt),
        "used_tool" => Some(EdgeKind::UsedTool),
        _ => None,
    }
}
