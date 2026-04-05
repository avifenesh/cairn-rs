//! Push-based inter-agent mailbox delivery (GAP-004).
//!
//! `MailboxDeliveryService` adds content-aware delivery on top of the
//! durable `MailboxService`. `MailboxWatcher` polls for deferred messages
//! whose delivery time has arrived.

use std::sync::Arc;
use cairn_domain::{MailboxMessageId, ProjectKey, RunId, TaskId};
use cairn_store::projections::{MailboxReadModel, MailboxRecord};

use crate::error::RuntimeError;
use crate::mailbox::MailboxService;

/// Higher-level delivery service for inter-agent messages.
///
/// Wraps `MailboxService` with content-aware delivery:
/// - `deliver()` — immediate delivery: appends the message and returns the record.
/// - `schedule()` — deferred delivery: stores with `deliver_at_ms > now`, watcher picks it up.
pub struct MailboxDeliveryService<M> {
    mailbox: Arc<M>,
}

impl<M: MailboxService> MailboxDeliveryService<M> {
    pub fn new(mailbox: Arc<M>) -> Self {
        Self { mailbox }
    }

    /// Deliver a message from one run to another immediately.
    pub async fn deliver(
        &self,
        project: &ProjectKey,
        from_run_id: Option<RunId>,
        to_run_id: Option<RunId>,
        to_task_id: Option<TaskId>,
        content: String,
    ) -> Result<MailboxRecord, RuntimeError> {
        let message_id = MailboxMessageId::new(format!("msg_{}", uuid_like()));
        self.mailbox
            .append(project, message_id, to_run_id, to_task_id, content, from_run_id, 0)
            .await
    }

    /// Schedule a message for deferred delivery at `deliver_at_ms`.
    pub async fn schedule(
        &self,
        project: &ProjectKey,
        from_run_id: Option<RunId>,
        to_run_id: Option<RunId>,
        to_task_id: Option<TaskId>,
        content: String,
        deliver_at_ms: u64,
    ) -> Result<MailboxRecord, RuntimeError> {
        let message_id = MailboxMessageId::new(format!("msg_{}", uuid_like()));
        self.mailbox
            .append(project, message_id, to_run_id, to_task_id, content, from_run_id, deliver_at_ms)
            .await
    }
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{:x}_{:x}", t.as_secs(), t.subsec_nanos())
}

/// Background watcher that polls for deferred mailbox messages.
///
/// Messages stored with `deliver_at_ms > 0` and `deliver_at_ms <= now_ms`
/// are considered "due." `flush_due()` returns the count of due messages found.
pub struct MailboxWatcher<S> {
    store: Arc<S>,
}

impl<S> MailboxWatcher<S>
where
    S: MailboxReadModel + Send + Sync + 'static,
{
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    /// Return messages due for delivery at or before `now_ms`.
    pub async fn due_messages(&self, now_ms: u64) -> Result<Vec<MailboxRecord>, RuntimeError> {
        self.store
            .list_pending(now_ms, 100)
            .await
            .map_err(RuntimeError::Store)
    }
}
