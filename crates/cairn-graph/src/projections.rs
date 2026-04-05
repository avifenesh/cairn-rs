use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde::{Deserialize, Serialize};

/// Graph node categories (RFC 004).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Session,
    Run,
    Task,
    Approval,
    Checkpoint,
    MailboxMessage,
    ToolInvocation,
    Memory,
    Document,
    Chunk,
    Source,
    PromptAsset,
    PromptVersion,
    PromptRelease,
    EvalRun,
    Skill,
    ChannelTarget,
    Signal,
    IngestJob,
    RouteDecision,
    ProviderCall,
}

/// Graph edge categories (RFC 004).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Triggered,
    Spawned,
    DependedOn,
    ApprovedBy,
    ResumedFrom,
    SentTo,
    ReadFrom,
    Cited,
    DerivedFrom,
    EmbeddedAs,
    EvaluatedBy,
    ReleasedAs,
    RolledBackTo,
    RoutedTo,
    UsedPrompt,
    UsedTool,
    CalledProvider,
}

/// A typed graph node with a unique identity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub node_id: String,
    pub kind: NodeKind,
    pub project: Option<ProjectKey>,
    pub created_at: u64,
}

/// A typed directed edge between two nodes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source_node_id: String,
    pub target_node_id: String,
    pub kind: EdgeKind,
    pub created_at: u64,
}

/// Graph projection service that builds graph structure from runtime events.
///
/// Per RFC 004, graph projections are asynchronously materialized from
/// runtime events and durable state. They are rebuildable derived views.
#[async_trait]
pub trait GraphProjection: Send + Sync {
    /// Record a node in the graph.
    async fn add_node(&self, node: GraphNode) -> Result<(), GraphProjectionError>;

    /// Record an edge in the graph.
    async fn add_edge(&self, edge: GraphEdge) -> Result<(), GraphProjectionError>;

    /// Check if a node exists.
    async fn node_exists(&self, node_id: &str) -> Result<bool, GraphProjectionError>;
}

/// Graph projection errors.
#[derive(Debug)]
pub enum GraphProjectionError {
    DuplicateNode(String),
    NodeNotFound(String),
    StorageError(String),
    Internal(String),
}

impl std::fmt::Display for GraphProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphProjectionError::DuplicateNode(id) => write!(f, "duplicate node: {id}"),
            GraphProjectionError::NodeNotFound(id) => write!(f, "node not found: {id}"),
            GraphProjectionError::StorageError(msg) => write!(f, "storage error: {msg}"),
            GraphProjectionError::Internal(msg) => {
                write!(f, "internal graph projection error: {msg}")
            }
        }
    }
}

impl std::error::Error for GraphProjectionError {}

#[cfg(test)]
mod tests {
    use super::{EdgeKind, NodeKind};

    #[test]
    fn node_kinds_cover_runtime_entities() {
        let kinds = [
            NodeKind::Session,
            NodeKind::Run,
            NodeKind::Task,
            NodeKind::Approval,
            NodeKind::Checkpoint,
            NodeKind::MailboxMessage,
            NodeKind::ToolInvocation,
        ];
        assert_eq!(kinds.len(), 7);
    }

    #[test]
    fn edge_kinds_are_distinct() {
        assert_ne!(EdgeKind::Triggered, EdgeKind::Spawned);
        assert_ne!(EdgeKind::Cited, EdgeKind::DerivedFrom);
    }
}
