//! Orchestrator error type.

use cairn_domain::{ApprovalId, TaskId};
use cairn_runtime::error::RuntimeError;
use cairn_store::StoreError;

/// All errors that can stop the orchestration loop.
#[derive(Debug)]
pub enum OrchestratorError {
    /// Gather phase failed (memory query, event read, etc.).
    Gather(String),
    /// Decide phase failed (LLM call, prompt resolution, JSON parse).
    Decide(String),
    /// Execute phase failed (tool dispatch, service call).
    Execute(String),
    /// A runtime service returned an error.
    Runtime(RuntimeError),
    /// The durable store returned an error.
    Store(StoreError),
    /// Iteration cap reached.
    MaxIterations { limit: u32 },
    /// Wall-clock timeout expired.
    Timeout,
    /// An operator denied an approval that was required to continue.
    ApprovalDenied { approval_id: ApprovalId },
    /// A dependency (subagent) that was blocking the run failed.
    DependencyFailed { child_task_id: TaskId },
    /// Memory retrieval or ingestion failed.
    Memory(String),
    /// Graph query failed.
    Graph(String),
}

impl std::fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrchestratorError::Gather(msg)   => write!(f, "gather error: {msg}"),
            OrchestratorError::Decide(msg)   => write!(f, "decide error: {msg}"),
            OrchestratorError::Execute(msg)  => write!(f, "execute error: {msg}"),
            OrchestratorError::Runtime(e)    => write!(f, "runtime error: {e}"),
            OrchestratorError::Store(e)      => write!(f, "store error: {e}"),
            OrchestratorError::MaxIterations { limit } =>
                write!(f, "max iterations reached: {limit}"),
            OrchestratorError::Timeout       => write!(f, "orchestration timed out"),
            OrchestratorError::ApprovalDenied { approval_id } =>
                write!(f, "approval denied: {approval_id}"),
            OrchestratorError::DependencyFailed { child_task_id } =>
                write!(f, "dependency failed: child task {child_task_id}"),
            OrchestratorError::Memory(msg)   => write!(f, "memory error: {msg}"),
            OrchestratorError::Graph(msg)    => write!(f, "graph error: {msg}"),
        }
    }
}

impl std::error::Error for OrchestratorError {}

impl From<RuntimeError> for OrchestratorError {
    fn from(e: RuntimeError) -> Self {
        OrchestratorError::Runtime(e)
    }
}

impl From<StoreError> for OrchestratorError {
    fn from(e: StoreError) -> Self {
        OrchestratorError::Store(e)
    }
}
