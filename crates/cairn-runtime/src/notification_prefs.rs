//! RFC 008: operator notification preferences service boundary.

use async_trait::async_trait;
use cairn_domain::notification_prefs::{
    NotificationChannel, NotificationPreference, NotificationRecord,
};
use cairn_domain::TenantId;

use crate::error::RuntimeError;

#[async_trait]
pub trait NotificationService: Send + Sync {
    /// Store or replace the full notification preference for an operator.
    async fn set_preferences(
        &self,
        tenant_id: TenantId,
        operator_id: String,
        event_types: Vec<String>,
        channels: Vec<NotificationChannel>,
    ) -> Result<(), RuntimeError>;

    /// Retrieve the notification preference for an operator.
    async fn get_preferences(
        &self,
        tenant_id: &TenantId,
        operator_id: &str,
    ) -> Result<Option<NotificationPreference>, RuntimeError>;

    /// Check if any operator in the tenant is subscribed to `event_type`;
    /// if so, emit a NotificationSent event for each matching channel.
    async fn notify_if_applicable(
        &self,
        tenant_id: &TenantId,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<Vec<NotificationRecord>, RuntimeError>;

    /// List all sent notifications for a tenant since a timestamp.
    async fn list_sent(
        &self,
        tenant_id: &TenantId,
        since_ms: u64,
    ) -> Result<Vec<NotificationRecord>, RuntimeError>;

    /// List notifications where delivery failed (delivered=false).
    async fn list_failed(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<NotificationRecord>, RuntimeError>;

    /// Retry delivery of a failed notification by record_id.
    async fn retry(
        &self,
        tenant_id: &TenantId,
        record_id: &str,
    ) -> Result<NotificationRecord, RuntimeError>;
}
