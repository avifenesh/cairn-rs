//! Tool-call approval projection (PR BP-2).
//!
//! Persists the current state of tool-call approval workflows driven by
//! the four domain events added in PR BP-1:
//!
//! * [`cairn_domain::events::ToolCallProposed`] — inserts a new
//!   `Pending` record.
//! * [`cairn_domain::events::ToolCallAmended`] — updates the amended
//!   arguments and `last_amended_at_ms`; the state remains `Pending`
//!   because the operator has not yet resolved the call.
//! * [`cairn_domain::events::ToolCallApproved`] — transitions the state
//!   to `Approved`, populating `operator_id`, `scope`, and
//!   `approved_tool_args` when the operator attached an override.
//! * [`cairn_domain::events::ToolCallRejected`] — transitions the state
//!   to `Rejected`, populating `operator_id` and `reason`.
//!
//! The read model exposes by-id, by-run, by-session, and
//! pending-by-project queries mirroring the shape of the legacy
//! [`crate::projections::ApprovalReadModel`].
//!
//! The `Timeout` state is reserved for a later PR that wires the runtime
//! timeout path (PR BP-3). Projections do not emit `Timeout` directly —
//! it is a placeholder so downstream surfaces can already plan for it.

use async_trait::async_trait;
use cairn_domain::{
    ApprovalMatchPolicy, ApprovalScope, OperatorId, ProjectKey, RunId, SessionId, ToolCallId,
};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Resolution state of a tool-call approval record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallApprovalState {
    /// `ToolCallProposed` has landed; no resolution yet.
    Pending,
    /// `ToolCallApproved` has landed.
    Approved,
    /// `ToolCallRejected` has landed.
    Rejected,
    /// Reserved for PR BP-3: the runtime timed the approval out before
    /// any operator decision landed. Not emitted by the projection
    /// in this PR; included so downstream surfaces (UI, SSE clients)
    /// can already pattern-match on the full set.
    Timeout,
}

impl ToolCallApprovalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Timeout => "timeout",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "timeout" => Ok(Self::Timeout),
            other => Err(StoreError::Internal(format!(
                "unknown tool_call_approval state: {other}"
            ))),
        }
    }
}

/// Current-state record for a tool-call approval.
///
/// Populated from the four `ToolCall*` events in event-log order. The
/// "effective executed args" invariant documented on
/// [`cairn_domain::events::ToolCallApproved`] is preserved by keeping
/// the three argument slots separate (`original_tool_args`,
/// `amended_tool_args`, `approved_tool_args`) — the runtime picks the
/// last populated one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallApprovalRecord {
    pub call_id: ToolCallId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub project: ProjectKey,
    pub tool_name: String,
    pub original_tool_args: serde_json::Value,
    /// `Some` after a `ToolCallAmended` event; captures the most recent
    /// amendment payload.
    pub amended_tool_args: Option<serde_json::Value>,
    /// `Some` after a `ToolCallApproved` event whose
    /// `approved_tool_args` field was `Some`.
    pub approved_tool_args: Option<serde_json::Value>,
    pub display_summary: Option<String>,
    pub match_policy: ApprovalMatchPolicy,
    pub state: ToolCallApprovalState,
    pub operator_id: Option<OperatorId>,
    pub scope: Option<ApprovalScope>,
    pub reason: Option<String>,
    pub proposed_at_ms: u64,
    pub approved_at_ms: Option<u64>,
    pub rejected_at_ms: Option<u64>,
    pub last_amended_at_ms: Option<u64>,
    pub version: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Read-model for tool-call approval current state.
#[async_trait]
pub trait ToolCallApprovalReadModel: Send + Sync {
    /// Fetch the projection record for a single tool-call id.
    async fn get(&self, call_id: &ToolCallId)
        -> Result<Option<ToolCallApprovalRecord>, StoreError>;

    /// List every tool-call approval for a run, oldest-first.
    async fn list_for_run(&self, run_id: &RunId)
        -> Result<Vec<ToolCallApprovalRecord>, StoreError>;

    /// List every tool-call approval for a session, oldest-first.
    async fn list_for_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ToolCallApprovalRecord>, StoreError>;

    /// List pending tool-call approvals for a project (operator inbox).
    /// Ordered by `(proposed_at_ms, call_id)` so replay is deterministic.
    async fn list_pending_for_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ToolCallApprovalRecord>, StoreError>;
}
