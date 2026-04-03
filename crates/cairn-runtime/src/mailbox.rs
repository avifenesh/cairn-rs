//! Mailbox service boundary per RFC 002.
//!
//! Mailbox messages are durable runtime records for coordination.
//! Durability belongs to the Rust runtime store, not a sidecar queue.

use async_trait::async_trait;
use cairn_domain::{MailboxMessageId, ProjectKey, RunId, TaskId};
use cairn_store::projections::MailboxRecord;

use crate::error::RuntimeError;

/// Mailbox service boundary.
///
/// Per RFC 002:
/// - mailbox durability belongs to the Rust runtime store
/// - any queue or sidecar transport is non-canonical
#[async_trait]
pub trait MailboxService: Send + Sync {
    /// Append a message to the mailbox.
    async fn append(
        &self,
        project: &ProjectKey,
        message_id: MailboxMessageId,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
    ) -> Result<MailboxRecord, RuntimeError>;

    /// Get a message by ID.
    async fn get(
        &self,
        message_id: &MailboxMessageId,
    ) -> Result<Option<MailboxRecord>, RuntimeError>;

    /// List messages linked to a run.
    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, RuntimeError>;

    /// List messages linked to a task.
    async fn list_by_task(
        &self,
        task_id: &TaskId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, RuntimeError>;
}
