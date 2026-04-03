use async_trait::async_trait;
use cairn_domain::{MailboxMessageId, ProjectKey, RunId, TaskId};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Current-state record for a mailbox message.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailboxRecord {
    pub message_id: MailboxMessageId,
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub version: u64,
    pub created_at: u64,
}

/// Read-model for mailbox current state.
#[async_trait]
pub trait MailboxReadModel: Send + Sync {
    async fn get(&self, message_id: &MailboxMessageId)
        -> Result<Option<MailboxRecord>, StoreError>;

    /// List messages linked to a run or task (mailbox inbox).
    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError>;

    async fn list_by_task(
        &self,
        task_id: &TaskId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError>;
}
