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
    /// FF stream-frame sink failed (best-effort telemetry channel).
    ///
    /// Emitted by the `TaskFrameSink` blanket impl when a
    /// `CairnTask::log_*` / `save_checkpoint` FCALL fails. The loop
    /// runner logs this at WARN and continues — frame writes are
    /// advisory, never fatal to a run. See
    /// `docs/design/CAIRN-FABRIC-FINALIZED.md` §4.5 for the semantics
    /// note.
    FrameSink(String),
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
    /// Every model on every binding in the DECIDE-phase routed chain
    /// failed. The attempt list captures per-model reasons so the app
    /// layer can surface a single `ToolCallApprovalService` proposal
    /// with actionable context.
    AllProvidersExhausted {
        attempts: Vec<cairn_runtime::FallbackAttempt>,
    },
    /// Provider credentials rejected the request (401/403 or equivalent).
    /// Retrying on another model won't help — the operator must rotate
    /// the credential. Short-circuits the fallback chain.
    ProviderAuthFailed {
        binding_id: String,
        model_id: String,
        detail: String,
    },
    /// Provider rejected the request (400-class other than 401/403/429).
    /// Retrying on another model won't help — cairn constructed a bad
    /// request. Short-circuits the fallback chain.
    ProviderInvalidRequest {
        binding_id: String,
        model_id: String,
        detail: String,
    },
}

impl std::fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrchestratorError::Gather(msg) => write!(f, "gather error: {msg}"),
            OrchestratorError::Decide(msg) => write!(f, "decide error: {msg}"),
            OrchestratorError::Execute(msg) => write!(f, "execute error: {msg}"),
            OrchestratorError::FrameSink(msg) => write!(f, "frame sink error: {msg}"),
            OrchestratorError::Runtime(e) => write!(f, "runtime error: {e}"),
            OrchestratorError::Store(e) => write!(f, "store error: {e}"),
            OrchestratorError::MaxIterations { limit } => {
                write!(f, "max iterations reached: {limit}")
            }
            OrchestratorError::Timeout => write!(f, "orchestration timed out"),
            OrchestratorError::ApprovalDenied { approval_id } => {
                write!(f, "approval denied: {approval_id}")
            }
            OrchestratorError::DependencyFailed { child_task_id } => {
                write!(f, "dependency failed: child task {child_task_id}")
            }
            OrchestratorError::Memory(msg) => write!(f, "memory error: {msg}"),
            OrchestratorError::Graph(msg) => write!(f, "graph error: {msg}"),
            OrchestratorError::AllProvidersExhausted { attempts } => {
                write!(
                    f,
                    "{}",
                    cairn_runtime::format_attempt_summary(attempts)
                )
            }
            OrchestratorError::ProviderAuthFailed {
                binding_id,
                model_id,
                detail,
            } => write!(
                f,
                "provider auth failed on binding={binding_id} model={model_id}: {detail}"
            ),
            OrchestratorError::ProviderInvalidRequest {
                binding_id,
                model_id,
                detail,
            } => write!(
                f,
                "provider rejected request on binding={binding_id} model={model_id}: {detail}"
            ),
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
