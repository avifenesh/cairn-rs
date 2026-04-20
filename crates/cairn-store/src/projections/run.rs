use async_trait::async_trait;
use cairn_domain::{
    FailureClass, PauseReason, ProjectKey, PromptReleaseId, ResumeTrigger, RunId, RunState,
    SessionId,
};
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
    pub prompt_release_id: Option<PromptReleaseId>,
    /// GAP-011: role attached at run creation (e.g. "researcher", "executor").
    #[serde(default)]
    pub agent_role_id: Option<String>,
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

    /// RFC 010: list non-terminal (active) runs across ALL sessions in a project.
    ///
    /// Operators must be able to view active runs regardless of which session
    /// originated them — session membership is irrelevant to the control-plane
    /// view.
    async fn list_active_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError>;

    /// List child runs of `parent_run_id`, ordered by `created_at` ASC.
    ///
    /// Indexed: Postgres/SQLite use `idx_runs_parent` (partial index on
    /// `parent_run_id WHERE NOT NULL`); InMemoryStore filters the live
    /// map. Replaces the pre-existing pattern of scanning 10 000
    /// `EventLog` entries for `RunCreated.parent_run_id == …` and
    /// silently truncating at the window — at high event rates the scan
    /// missed children created before the window rolled off. Callers
    /// get every child with no truncation.
    async fn list_by_parent_run(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError>;
}
