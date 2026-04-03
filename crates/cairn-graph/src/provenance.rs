use async_trait::async_trait;
use cairn_domain::{
    CheckpointId, KnowledgeDocumentId, PromptReleaseId, RunId, SessionId, SourceId, TaskId,
    ToolInvocationId,
};
use serde::{Deserialize, Serialize};

use crate::projections::NodeKind;

/// A provenance chain link, following edges from an outcome back to its causes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProvenanceLink {
    pub node_id: String,
    pub kind: NodeKind,
    pub depth: u32,
}

/// Full provenance chain for operator "why did this happen?" surfaces.
#[derive(Clone, Debug)]
pub struct ProvenanceChain {
    pub root_node_id: String,
    pub links: Vec<ProvenanceLink>,
}

/// Execution provenance: which runs/tasks/tools contributed to an outcome.
#[derive(Clone, Debug)]
pub struct ExecutionProvenance {
    pub session_id: SessionId,
    pub run_ids: Vec<RunId>,
    pub task_ids: Vec<TaskId>,
    pub tool_invocation_ids: Vec<ToolInvocationId>,
    pub checkpoint_ids: Vec<CheckpointId>,
    pub prompt_release_ids: Vec<PromptReleaseId>,
}

/// Retrieval provenance: which sources/documents/chunks were used.
#[derive(Clone, Debug)]
pub struct RetrievalProvenance {
    pub source_ids: Vec<SourceId>,
    pub document_ids: Vec<KnowledgeDocumentId>,
    pub chunk_ids: Vec<String>,
}

/// Provenance service boundary.
///
/// Per RFC 004, the graph must support explaining why a result or action
/// happened, showing provenance for memory and retrieval, and exposing
/// dependencies between prompts, skills, tools, and outcomes.
#[async_trait]
pub trait ProvenanceService: Send + Sync {
    /// Trace execution provenance for a session or run.
    async fn execution_provenance(
        &self,
        session_id: &SessionId,
    ) -> Result<ExecutionProvenance, ProvenanceError>;

    /// Trace retrieval provenance for a specific answer/output node.
    async fn retrieval_provenance(
        &self,
        answer_node_id: &str,
    ) -> Result<RetrievalProvenance, ProvenanceError>;

    /// Build a full provenance chain from an outcome node back to root causes.
    async fn provenance_chain(
        &self,
        node_id: &str,
        max_depth: u32,
    ) -> Result<ProvenanceChain, ProvenanceError>;
}

/// Provenance-specific errors.
#[derive(Debug)]
pub enum ProvenanceError {
    NodeNotFound(String),
    ChainTooDeep { max: u32 },
    StorageError(String),
    Internal(String),
}

impl std::fmt::Display for ProvenanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProvenanceError::NodeNotFound(id) => write!(f, "provenance node not found: {id}"),
            ProvenanceError::ChainTooDeep { max } => {
                write!(f, "provenance chain exceeded max depth {max}")
            }
            ProvenanceError::StorageError(msg) => write!(f, "storage error: {msg}"),
            ProvenanceError::Internal(msg) => write!(f, "internal provenance error: {msg}"),
        }
    }
}

impl std::error::Error for ProvenanceError {}
