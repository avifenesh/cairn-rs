use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{SessionReadModel, SessionRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
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
    S: EventLog + SessionReadModel + 'static,
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
