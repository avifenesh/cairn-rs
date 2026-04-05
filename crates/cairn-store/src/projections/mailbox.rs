use async_trait::async_trait;
use cairn_domain::{MailboxMessageId, ProjectKey, RunId, TaskId};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Maximum content length per message (mirrors cairn Go `maxMessageContentLen = 4000`).
pub const MAX_MESSAGE_CONTENT_LEN: usize = 4000;

/// Current-state record for a mailbox message.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailboxRecord {
    pub message_id: MailboxMessageId,
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    /// Task or session that sent this message (`from` in the inter-agent protocol).
    #[serde(default)]
    pub from_task_id: Option<TaskId>,
    /// Message content, truncated to `MAX_MESSAGE_CONTENT_LEN` chars.
    #[serde(default)]
    pub content: String,
    /// Run that sent this message (inter-agent delivery).
    #[serde(default)]
    pub from_run_id: Option<RunId>,
    /// Epoch-ms timestamp after which this message should be delivered; 0 means immediate.
    #[serde(default)]
    pub deliver_at_ms: u64,
    /// RFC 002: display name or agent ID of the message sender.
    #[serde(default)]
    pub sender: Option<String>,
    /// RFC 002: display name or agent ID of the intended recipient.
    #[serde(default)]
    pub recipient: Option<String>,
    /// RFC 002: full message body.
    #[serde(default)]
    pub body: Option<String>,
    /// RFC 002: epoch-ms when the message was sent by the sender.
    #[serde(default)]
    pub sent_at: Option<u64>,
    /// RFC 002: delivery lifecycle state.
    #[serde(default)]
    pub delivery_status: Option<String>,
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

    /// List messages whose `deliver_at_ms > 0` and `deliver_at_ms <= now_ms` (deferred delivery).
    async fn list_pending(&self, now_ms: u64, limit: usize) -> Result<Vec<MailboxRecord>, StoreError>;
}
