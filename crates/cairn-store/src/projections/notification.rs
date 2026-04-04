//! RFC 008: read-model for notification preferences and sent notifications.

use async_trait::async_trait;
use cairn_domain::notification_prefs::{NotificationPreference, NotificationRecord};
use cairn_domain::TenantId;

use crate::error::StoreError;

/// Read-model for operator notification preferences and audit records.
#[async_trait]
pub trait NotificationReadModel: Send + Sync {
    async fn get_preferences(
        &self,
        tenant_id: &TenantId,
        operator_id: &str,
    ) -> Result<Option<NotificationPreference>, StoreError>;

    async fn list_preferences_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<NotificationPreference>, StoreError>;

    async fn list_sent_notifications(
        &self,
        tenant_id: &TenantId,
        since_ms: u64,
    ) -> Result<Vec<NotificationRecord>, StoreError>;

    /// List notifications where delivery failed (delivered=false).
    async fn list_failed_notifications(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<NotificationRecord>, StoreError>;
}
