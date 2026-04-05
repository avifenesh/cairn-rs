use async_trait::async_trait;
use cairn_domain::{ProjectKey, SessionId, SessionState};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Current-state record for a session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: SessionId,
    pub project: ProjectKey,
    pub state: SessionState,
    pub version: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Read-model for session current state.
#[async_trait]
pub trait SessionReadModel: Send + Sync {
    async fn get(&self, session_id: &SessionId) -> Result<Option<SessionRecord>, StoreError>;

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SessionRecord>, StoreError>;

    /// RFC fleet view: list non-terminal sessions across all projects in a workspace.
    ///
    /// Returns at most `limit` sessions sorted by `updated_at` descending.
    /// Used by `GET /v1/fleet` to enumerate active agent sessions.
    async fn list_active(
        &self,
        limit: usize,
    ) -> Result<Vec<SessionRecord>, StoreError>;
}
