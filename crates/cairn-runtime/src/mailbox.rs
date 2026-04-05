//! Mailbox service boundary per RFC 002 + GAP-004 push-based inter-agent coordination.
//!
//! Mailbox messages are durable runtime records for coordination.
//! Durability belongs to the Rust runtime store, not a sidecar queue.
//!
//! The `send`/`receive` extension mirrors `cairn/internal/agent/mailbox.go`:
//! - `send(from, to, content)` truncates content to 4000 chars and appends durably.
//! - `receive(to)` reads all messages for the recipient task.
//! - `format_for_injection` produces a text block suitable for LLM context injection.

use async_trait::async_trait;
use cairn_domain::{MailboxMessageId, ProjectKey, RunId, TaskId};
use cairn_store::projections::{MailboxRecord, MAX_MESSAGE_CONTENT_LEN};

use crate::error::RuntimeError;

/// Mailbox service boundary.
///
/// Per RFC 002:
/// - mailbox durability belongs to the Rust runtime store
/// - any queue or sidecar transport is non-canonical
#[async_trait]
pub trait MailboxService: Send + Sync {
    /// Append a message to the mailbox (low-level, for system use).
    async fn append(
        &self,
        project: &ProjectKey,
        message_id: MailboxMessageId,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
        content: String,
        from_run_id: Option<RunId>,
        deliver_at_ms: u64,
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

    /// Push a content message from one task to another (GAP-004 inter-agent mailbox).
    ///
    /// - `from` is the sender's task ID (recorded on the message for context).
    /// - `to` is the recipient's task ID (message is retrievable via `receive`).
    /// - `content` is truncated to `MAX_MESSAGE_CONTENT_LEN` chars.
    ///
    /// Mirrors `cairn/internal/agent/mailbox.go` `Send()`.
    async fn send(
        &self,
        project: &ProjectKey,
        from: TaskId,
        to: TaskId,
        content: String,
    ) -> Result<MailboxRecord, RuntimeError>;

    /// Retrieve all messages addressed to `task_id`.
    ///
    /// Returns messages in chronological order (oldest first).
    /// Mirrors `cairn/internal/agent/mailbox.go` `Receive()` (read-only; callers
    /// must track their own read cursor to avoid re-processing).
    async fn receive(
        &self,
        task_id: &TaskId,
        limit: usize,
    ) -> Result<Vec<MailboxRecord>, RuntimeError>;
}

/// Truncate message content to `MAX_MESSAGE_CONTENT_LEN`, appending a marker if cut.
///
/// Mirrors `cairn/internal/agent/mailbox.go` content truncation.
pub fn truncate_message_content(content: &str) -> String {
    if content.len() <= MAX_MESSAGE_CONTENT_LEN {
        return content.to_owned();
    }
    // Truncate at a char boundary.
    let truncated: String = content.chars().take(MAX_MESSAGE_CONTENT_LEN).collect();
    format!("{}... (truncated)", truncated)
}

/// Format mailbox messages for injection into an LLM prompt.
///
/// Returns an empty string if `messages` is empty.
/// Mirrors `cairn/internal/agent/mailbox.go` `FormatForInjection()`.
pub fn format_for_injection(messages: &[MailboxRecord]) -> String {
    if messages.is_empty() {
        return String::new();
    }
    let mut lines = vec!["[Inter-agent messages]".to_owned()];
    for msg in messages {
        let from = msg
            .from_task_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or("unknown");
        let content = msg.content.as_str();
        lines.push(format!("From {from}: {content}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_content_unchanged() {
        let s = "hello world";
        assert_eq!(truncate_message_content(s), s);
    }

    #[test]
    fn truncate_long_content_appends_marker() {
        let long: String = "x".repeat(MAX_MESSAGE_CONTENT_LEN + 100);
        let result = truncate_message_content(&long);
        assert!(result.len() <= MAX_MESSAGE_CONTENT_LEN + 20, "truncated string must be near limit");
        assert!(result.ends_with("... (truncated)"), "must append truncation marker");
        assert!(!result.contains(&"x".repeat(MAX_MESSAGE_CONTENT_LEN + 1)));
    }

    #[test]
    fn truncate_exactly_at_limit_unchanged() {
        let s: String = "a".repeat(MAX_MESSAGE_CONTENT_LEN);
        assert_eq!(truncate_message_content(&s), s);
    }

    #[test]
    fn format_for_injection_empty_is_empty_string() {
        assert_eq!(format_for_injection(&[]), "");
    }

    #[test]
    fn format_for_injection_includes_header_and_messages() {
        use cairn_domain::{ProjectKey, TaskId};
        let record = MailboxRecord {
            message_id: cairn_domain::MailboxMessageId::new("msg_1"),
            project: ProjectKey::new("t", "w", "p"),
            run_id: None,
            task_id: Some(TaskId::new("to_task")),
            from_task_id: Some(TaskId::new("from_task")),
            content: "Hello from orchestrator".to_owned(),
            from_run_id: None,
            deliver_at_ms: 0,
            sender: None,
            recipient: None,
            body: None,
            sent_at: None,
            delivery_status: None,
            version: 1,
            created_at: 1000,
        };
        let result = format_for_injection(&[record]);
        assert!(result.starts_with("[Inter-agent messages]"), "must start with header");
        assert!(result.contains("From from_task: Hello from orchestrator"));
    }
}
