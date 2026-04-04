//! RFC 008: operator notification preferences domain types.

use crate::TenantId;
use serde::{Deserialize, Serialize};

/// A channel through which a notification is delivered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationChannel {
    /// Channel kind: "email", "webhook", or "slack".
    pub kind: String,
    /// Target address: email address, webhook URL, or Slack channel/webhook.
    pub target: String,
}

/// Stored preference: which event types an operator wants to be notified about
/// and through which channels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPreference {
    pub pref_id: String,
    pub tenant_id: TenantId,
    pub operator_id: String,
    pub event_types: Vec<String>,
    pub channels: Vec<NotificationChannel>,
}

/// Audit record of a notification that was dispatched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationRecord {
    pub record_id: String,
    pub tenant_id: TenantId,
    pub operator_id: String,
    pub event_type: String,
    pub channel_kind: String,
    pub channel_target: String,
    pub payload: serde_json::Value,
    pub sent_at_ms: u64,
    /// Whether the delivery call succeeded (webhook POST, etc.).
    pub delivered: bool,
    /// If delivery failed, the error message.
    pub delivery_error: Option<String>,
}
