//! `notify_operator` built-in tool — operator notification with mailbox + SSE.
//!
//! Agents use this to escalate messages, signal completion, or surface
//! problems that need human attention.  Delivery is two-path:
//!
//! 1. **Mailbox** (durable): message written to `MailboxService` → survives
//!    restart, retrievable via `GET /v1/runs/:id/mailbox`.
//! 2. **Realtime** (optional): fires an `Arc<dyn NotificationSink>` so
//!    cairn-app can forward the message as an SSE event to connected dashboards.
//!
//! ## Parameters
//! ```json
//! {
//!   "message":   "Blocked: ambiguous requirement in step 3.",
//!   "severity":  "warning",    // "info" | "warning" | "critical" (default: "info")
//!   "channel":   "operator"    // freeform routing tag (default: "operator")
//! }
//! ```
//!
//! ## Response
//! ```json
//! { "delivered": true, "channel": "operator", "severity": "warning", "mailbox_id": "msg_…" }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{MailboxMessageId, ProjectKey};
use cairn_runtime::MailboxService;
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

// ── NotificationSink ──────────────────────────────────────────────────────────

/// Pluggable realtime delivery channel (SSE, webhook, etc.).
///
/// cairn-app wires a concrete `SseSink` at construction time.
/// cairn-tools defines only the trait to avoid a cairn-app dependency.
#[async_trait]
pub trait NotificationSink: Send + Sync {
    async fn emit(&self, channel: &str, severity: &str, message: &str);
}

/// No-op sink used when no realtime channel is configured.
pub struct NoopSink;
#[async_trait]
impl NotificationSink for NoopSink {
    async fn emit(&self, _channel: &str, _severity: &str, _message: &str) {}
}

// ── Severity ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity { Info, Warning, Critical }

impl Severity {
    fn from_str(s: &str) -> Self {
        match s {
            "warning"  => Self::Warning,
            "critical" => Self::Critical,
            _          => Self::Info,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Info     => "info",
            Self::Warning  => "warning",
            Self::Critical => "critical",
        }
    }
}

// ── NotifyOperatorTool ────────────────────────────────────────────────────────

/// Core-tier operator notification tool.
pub struct NotifyOperatorTool {
    mailbox: Option<Arc<dyn MailboxService>>,
    sink:    Arc<dyn NotificationSink>,
}

impl NotifyOperatorTool {
    /// Create with both a durable mailbox and a realtime sink.
    pub fn new(
        mailbox: Option<Arc<dyn MailboxService>>,
        sink:    Arc<dyn NotificationSink>,
    ) -> Self {
        Self { mailbox, sink }
    }

    /// Stub constructor — no mailbox, no-op SSE (for tests / stub registry).
    pub fn stub() -> Self {
        Self { mailbox: None, sink: Arc::new(NoopSink) }
    }
}

impl Default for NotifyOperatorTool {
    fn default() -> Self { Self::stub() }
}

#[async_trait]
impl ToolHandler for NotifyOperatorTool {
    fn name(&self) -> &str { "notify_operator" }

    fn tier(&self) -> ToolTier { ToolTier::Core }

    fn description(&self) -> &str {
        "Send a notification to the operator. \
         Use for escalations, status updates, and critical alerts. \
         Message is delivered durably via the mailbox and in realtime via SSE."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["message"],
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Notification text (max 4000 chars). \
                                    Be concise — operators read many notifications."
                },
                "severity": {
                    "type": "string",
                    "enum": ["info", "warning", "critical"],
                    "description": "Impact level. \
                                    info = FYI, warning = attention needed, \
                                    critical = immediate action required.",
                    "default": "info"
                },
                "channel": {
                    "type": "string",
                    "description": "Routing tag for the notification \
                                    (e.g. 'operator', 'on-call', run ID). \
                                    Defaults to 'operator'."
                }
            }
        })
    }

    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let message = args["message"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs {
                field:   "message".into(),
                message: "required string".into(),
            })?
            .trim();

        if message.is_empty() {
            return Err(ToolError::InvalidArgs {
                field:   "message".into(),
                message: "message must not be empty".into(),
            });
        }

        let severity = Severity::from_str(
            args["severity"].as_str().unwrap_or("info"),
        );
        let channel = args["channel"].as_str().unwrap_or("operator");

        // ── Path 1: Durable mailbox ───────────────────────────────────────────
        let mailbox_id = if let Some(ref svc) = self.mailbox {
            let msg_id = MailboxMessageId::new(format!(
                "notify_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
            ));

            let content = format!(
                "[{severity}] [{channel}] {message}",
                severity = severity.as_str(),
            );

            match svc.append(
                project,
                msg_id.clone(),
                None, // run_id: let MailboxServiceImpl derive from project context
                None, // task_id
                content,
                None, // from_run_id
                0,    // deliver_at_ms = immediate
            ).await {
                Ok(record) => Some(record.message_id.as_str().to_owned()),
                Err(e) => {
                    // Mailbox failure is non-fatal — still emit SSE and succeed
                    eprintln!("notify_operator: mailbox append failed: {e}");
                    None
                }
            }
        } else {
            None
        };

        // ── Path 2: Realtime SSE ──────────────────────────────────────────────
        self.sink.emit(channel, severity.as_str(), message).await;

        Ok(ToolResult::ok(serde_json::json!({
            "delivered":  true,
            "channel":    channel,
            "severity":   severity.as_str(),
            "mailbox_id": mailbox_id,
        })))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn project() -> ProjectKey { ProjectKey::new("t","w","p") }

    /// Sink that records calls for assertion.
    struct RecordingSink {
        calls: Mutex<Vec<(String, String, String)>>,
    }
    impl RecordingSink {
        fn new() -> Arc<Self> { Arc::new(Self { calls: Mutex::new(vec![]) }) }
        fn recorded(&self) -> Vec<(String, String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }
    #[async_trait]
    impl NotificationSink for RecordingSink {
        async fn emit(&self, ch: &str, sev: &str, msg: &str) {
            self.calls.lock().unwrap().push((ch.into(), sev.into(), msg.into()));
        }
    }

    // ── Schema ────────────────────────────────────────────────────────────────

    #[test]
    fn tier_is_core() {
        assert_eq!(NotifyOperatorTool::stub().tier(), ToolTier::Core);
    }

    #[test]
    fn schema_has_required_message() {
        let s = NotifyOperatorTool::stub().parameters_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("message")));
        // severity and channel must be optional
        assert!(!req.iter().any(|v| v.as_str() == Some("severity")));
        assert!(!req.iter().any(|v| v.as_str() == Some("channel")));
    }

    #[test]
    fn schema_severity_enum_has_three_values() {
        let s = NotifyOperatorTool::stub().parameters_schema();
        let enums = s["properties"]["severity"]["enum"].as_array().unwrap();
        assert_eq!(enums.len(), 3);
    }

    // ── Successful delivery ───────────────────────────────────────────────────

    #[tokio::test]
    async fn delivers_message_to_sink() {
        let sink = RecordingSink::new();
        let tool = NotifyOperatorTool::new(None, sink.clone());

        let res = tool.execute(&project(), serde_json::json!({
            "message": "Deploy succeeded",
        })).await.unwrap();

        assert_eq!(res.output["delivered"], true);
        assert_eq!(res.output["channel"], "operator");
        assert_eq!(res.output["severity"], "info");

        let calls = sink.recorded();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "operator");
        assert_eq!(calls[0].1, "info");
        assert!(calls[0].2.contains("Deploy succeeded"));
    }

    #[tokio::test]
    async fn severity_and_channel_respected() {
        let sink = RecordingSink::new();
        let tool = NotifyOperatorTool::new(None, sink.clone());

        let res = tool.execute(&project(), serde_json::json!({
            "message":  "Out of memory",
            "severity": "critical",
            "channel":  "on-call",
        })).await.unwrap();

        assert_eq!(res.output["severity"], "critical");
        assert_eq!(res.output["channel"],  "on-call");

        let calls = sink.recorded();
        assert_eq!(calls[0].0, "on-call");
        assert_eq!(calls[0].1, "critical");
    }

    #[tokio::test]
    async fn default_severity_is_info() {
        let sink = RecordingSink::new();
        let tool = NotifyOperatorTool::new(None, sink.clone());
        let res = tool.execute(&project(), serde_json::json!({
            "message": "status update"
        })).await.unwrap();
        assert_eq!(res.output["severity"], "info");
    }

    // ── Error cases ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn missing_message_is_invalid_args() {
        let err = NotifyOperatorTool::stub()
            .execute(&project(), serde_json::json!({}))
            .await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn empty_message_is_invalid_args() {
        let err = NotifyOperatorTool::stub()
            .execute(&project(), serde_json::json!({"message": "  "}))
            .await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    // ── Without mailbox (stub mode) ───────────────────────────────────────────

    #[tokio::test]
    async fn stub_mode_returns_delivered_without_mailbox_id() {
        let res = NotifyOperatorTool::stub()
            .execute(&project(), serde_json::json!({"message": "hello"}))
            .await.unwrap();
        assert_eq!(res.output["delivered"], true);
        // No mailbox configured → mailbox_id is null
        assert!(res.output["mailbox_id"].is_null());
    }

    // ── Severity helper ───────────────────────────────────────────────────────

    #[test]
    fn severity_round_trips() {
        for (s, expected) in [("info", Severity::Info), ("warning", Severity::Warning), ("critical", Severity::Critical)] {
            assert_eq!(Severity::from_str(s), expected);
            assert_eq!(Severity::from_str(s).as_str(), s);
        }
        assert_eq!(Severity::from_str("unknown"), Severity::Info, "unknown → info");
    }
}
