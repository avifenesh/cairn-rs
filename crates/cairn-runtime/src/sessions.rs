//! Session service boundary per RFC 005.
//!
//! Sessions are long-lived conversational or operational contexts.
//! Session state is derived from run outcomes plus explicit close/archive.

use async_trait::async_trait;
use cairn_domain::{ProjectKey, SessionId};
use cairn_store::projections::SessionRecord;

use crate::error::RuntimeError;

/// Session service boundary.
///
/// Per RFC 005:
/// - sessions start as `open`
/// - session state is derived from run outcomes
/// - sessions can be explicitly archived
#[async_trait]
pub trait SessionService: Send + Sync {
    /// Create a new session in a project.
    async fn create(
        &self,
        project: &ProjectKey,
        session_id: SessionId,
    ) -> Result<SessionRecord, RuntimeError>;

    /// Get a session by ID.
    async fn get(&self, session_id: &SessionId) -> Result<Option<SessionRecord>, RuntimeError>;

    /// List sessions for a project.
    async fn list(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SessionRecord>, RuntimeError>;

    /// Archive a session (terminal).
    async fn archive(&self, session_id: &SessionId) -> Result<SessionRecord, RuntimeError>;
}

#[cfg(test)]
mod tests {
    use cairn_domain::SessionId;

    #[test]
    fn session_id_is_stable() {
        let id = SessionId::new("sess_123");
        assert_eq!(id.as_str(), "sess_123");
    }
}
