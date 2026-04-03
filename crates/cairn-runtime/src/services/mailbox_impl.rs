use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{MailboxReadModel, MailboxRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::mailbox::MailboxService;

pub struct MailboxServiceImpl<S> {
    store: Arc<S>,
}

impl<S> MailboxServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> MailboxService for MailboxServiceImpl<S>
where
    S: EventLog + MailboxReadModel + 'static,
{
    async fn append(
        &self,
        project: &ProjectKey,
        message_id: MailboxMessageId,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
    ) -> Result<MailboxRecord, RuntimeError> {
        let event = make_envelope(RuntimeEvent::MailboxMessageAppended(
            MailboxMessageAppended {
                project: project.clone(),
                message_id: message_id.clone(),
                run_id,
                task_id,
            },
        ));

        self.store.append(&[event]).await?;

        MailboxReadModel::get(self.store.as_ref(), &message_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("mailbox message not found after append".into()))
    }

    async fn get(
        &self,
        message_id: &MailboxMessageId,
    ) -> Result<Option<MailboxRecord>, RuntimeError> {
        Ok(MailboxReadModel::get(self.store.as_ref(), message_id).await?)
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, RuntimeError> {
        Ok(self.store.list_by_run(run_id, limit, offset).await?)
    }

    async fn list_by_task(
        &self,
        task_id: &TaskId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, RuntimeError> {
        Ok(self.store.list_by_task(task_id, limit, offset).await?)
    }
}
