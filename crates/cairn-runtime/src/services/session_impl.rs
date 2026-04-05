use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{QuotaReadModel, RunReadModel, SessionReadModel, SessionRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use super::quota_impl::enforce_session_quota;
use crate::error::RuntimeError;
use crate::sessions::SessionService;

pub struct SessionServiceImpl<S> {
    store: Arc<S>,
}

impl<S> SessionServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> SessionService for SessionServiceImpl<S>
where
    S: EventLog + SessionReadModel + QuotaReadModel + 'static,
{
    async fn create(
        &self,
        project: &ProjectKey,
        session_id: SessionId,
    ) -> Result<SessionRecord, RuntimeError> {
        if SessionReadModel::get(self.store.as_ref(), &session_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "session",
                id: session_id.to_string(),
            });
        }

        // Enforce session quota before creating the session.
        enforce_session_quota(self.store.as_ref(), &project.tenant_id).await?;

        let event = make_envelope(RuntimeEvent::SessionCreated(SessionCreated {
            project: project.clone(),
            session_id: session_id.clone(),
        }));

        self.store.append(&[event]).await?;

        SessionReadModel::get(self.store.as_ref(), &session_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("session not found after create".into()))
    }

    async fn get(&self, session_id: &SessionId) -> Result<Option<SessionRecord>, RuntimeError> {
        Ok(SessionReadModel::get(self.store.as_ref(), session_id).await?)
    }

    async fn list(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SessionRecord>, RuntimeError> {
        Ok(self.store.list_by_project(project, limit, offset).await?)
    }

    async fn archive(&self, session_id: &SessionId) -> Result<SessionRecord, RuntimeError> {
        let session = SessionReadModel::get(self.store.as_ref(), session_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "session",
                id: session_id.to_string(),
            })?;

        if session.state == SessionState::Archived {
            return Ok(session);
        }

        let event = make_envelope(RuntimeEvent::SessionStateChanged(SessionStateChanged {
            project: session.project.clone(),
            session_id: session_id.clone(),
            transition: StateTransition {
                from: Some(session.state),
                to: SessionState::Archived,
            },
        }));

        self.store.append(&[event]).await?;

        SessionReadModel::get(self.store.as_ref(), session_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("session not found after archive".into()))
    }
}

/// Derive session outcome from run states and update if changed (RFC 005).
///
/// Called after a run reaches a terminal state. Uses `derive_session_state()`
/// to compute the expected session state from current runs, then emits
/// `SessionStateChanged` if the state actually changed.
pub async fn derive_and_update_session<S>(
    store: &S,
    session_id: &SessionId,
) -> Result<(), RuntimeError>
where
    S: EventLog + SessionReadModel + RunReadModel,
{
    let session = match SessionReadModel::get(store, session_id).await? {
        Some(s) => s,
        None => return Ok(()), // Session not found — nothing to derive.
    };

    // Already terminal — don't override.
    if session.state.is_terminal() {
        return Ok(());
    }

    let is_archived = session.state == SessionState::Archived;
    let any_non_terminal = store.any_non_terminal(session_id).await?;
    let latest_root = store.latest_root_run(session_id).await?;
    let latest_root_terminal = latest_root
        .filter(|r| r.state.is_terminal())
        .map(|r| r.state);

    let derived = derive_session_state(is_archived, any_non_terminal, latest_root_terminal);

    if derived != session.state {
        let event = make_envelope(RuntimeEvent::SessionStateChanged(SessionStateChanged {
            project: session.project.clone(),
            session_id: session_id.clone(),
            transition: StateTransition {
                from: Some(session.state),
                to: derived,
            },
        }));
        store.append(&[event]).await?;
    }

    Ok(())
}
