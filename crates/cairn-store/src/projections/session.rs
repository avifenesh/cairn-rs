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
}
