use async_trait::async_trait;
use cairn_domain::{FailureClass, PauseReason, ProjectKey, ResumeTrigger, RunId, RunState, SessionId};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Current-state record for a run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub parent_run_id: Option<RunId>,
    pub project: ProjectKey,
    pub state: RunState,
    pub failure_class: Option<FailureClass>,
    pub pause_reason: Option<PauseReason>,
    pub resume_trigger: Option<ResumeTrigger>,
    pub version: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Read-model for run current state.
#[async_trait]
pub trait RunReadModel: Send + Sync {
    async fn get(&self, run_id: &RunId) -> Result<Option<RunRecord>, StoreError>;

    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RunRecord>, StoreError>;

    /// List non-terminal runs in a session (used by session state derivation).
    async fn any_non_terminal(&self, session_id: &SessionId) -> Result<bool, StoreError>;

    /// Get the latest root run (no parent_run_id) in a session.
    async fn latest_root_run(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<RunRecord>, StoreError>;

    /// List runs in a specific state (used by recovery sweeps).
    async fn list_by_state(
        &self,
        state: RunState,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError>;
}
