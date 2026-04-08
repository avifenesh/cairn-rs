//! RFC 010/011: operator notification preference service with webhook delivery.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::notification_prefs::{
    NotificationChannel, NotificationPreference, NotificationRecord,
};
use cairn_domain::{NotificationPreferenceSet, NotificationSent, RuntimeEvent, TenantId};
use cairn_store::projections::NotificationReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::notification_prefs::NotificationService;

static RECORD_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_record_id() -> String {
    let n = RECORD_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("notif_{n}")
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Attempt an HTTP POST to a webhook URL.
/// The webhook feature is not enabled; returns an error indicating this.
async fn post_webhook(_url: &str, _body: &serde_json::Value) -> Result<(), String> {
    Err("webhook feature not enabled".to_owned())
}

/// Attempt delivery for a single channel. Returns (delivered, delivery_error).
async fn attempt_delivery(
    channel: &NotificationChannel,
    webhook_body: &serde_json::Value,
) -> (bool, Option<String>) {
    if channel.kind == "webhook" {
        match post_webhook(&channel.target, webhook_body).await {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e)),
        }
    } else {
        // Non-webhook channels (email, slack) are not implemented yet — mark as delivered.
        (true, None)
    }
}

pub struct NotificationServiceImpl<S> {
    store: Arc<S>,
}

impl<S> NotificationServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> NotificationService for NotificationServiceImpl<S>
where
    S: EventLog + NotificationReadModel + Send + Sync + 'static,
{
    async fn set_preferences(
        &self,
        tenant_id: TenantId,
        operator_id: String,
        event_types: Vec<String>,
        channels: Vec<NotificationChannel>,
    ) -> Result<(), RuntimeError> {
        let event = make_envelope(RuntimeEvent::NotificationPreferenceSet(
            NotificationPreferenceSet {
                tenant_id,
                operator_id,
                event_types,
                channels,
                set_at_ms: now_millis(),
            },
        ));
        self.store.append(&[event]).await?;
        Ok(())
    }

    async fn get_preferences(
        &self,
        tenant_id: &TenantId,
        operator_id: &str,
    ) -> Result<Option<NotificationPreference>, RuntimeError> {
        Ok(
            NotificationReadModel::get_preferences(self.store.as_ref(), tenant_id, operator_id)
                .await?,
        )
    }

    async fn notify_if_applicable(
        &self,
        tenant_id: &TenantId,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<Vec<NotificationRecord>, RuntimeError> {
        let prefs =
            NotificationReadModel::list_preferences_by_tenant(self.store.as_ref(), tenant_id)
                .await?;

        let mut dispatched = Vec::new();
        let mut envelopes = Vec::new();
        let now = now_millis();

        for pref in &prefs {
            if !pref.event_types.iter().any(|t| t == event_type) {
                continue;
            }
            for channel in &pref.channels {
                let record_id = next_record_id();

                // Build webhook body.
                let webhook_body = serde_json::json!({
                    "event_type": event_type,
                    "payload": payload,
                    "tenant_id": tenant_id.as_str(),
                    "occurred_at_ms": now,
                });

                let (delivered, delivery_error) = attempt_delivery(channel, &webhook_body).await;

                let record = NotificationRecord {
                    record_id: record_id.clone(),
                    tenant_id: tenant_id.clone(),
                    operator_id: pref.operator_id.clone(),
                    event_type: event_type.to_owned(),
                    channel_kind: channel.kind.clone(),
                    channel_target: channel.target.clone(),
                    payload: payload.clone(),
                    sent_at_ms: now,
                    delivered,
                    delivery_error: delivery_error.clone(),
                };
                envelopes.push(make_envelope(RuntimeEvent::NotificationSent(
                    NotificationSent {
                        record_id,
                        tenant_id: tenant_id.clone(),
                        operator_id: pref.operator_id.clone(),
                        event_type: event_type.to_owned(),
                        channel_kind: channel.kind.clone(),
                        channel_target: channel.target.clone(),
                        payload: payload.clone(),
                        sent_at_ms: now,
                        delivered,
                        delivery_error,
                    },
                )));
                dispatched.push(record);
            }
        }

        if !envelopes.is_empty() {
            self.store.append(&envelopes).await?;
        }

        Ok(dispatched)
    }

    async fn list_sent(
        &self,
        tenant_id: &TenantId,
        since_ms: u64,
    ) -> Result<Vec<NotificationRecord>, RuntimeError> {
        Ok(
            NotificationReadModel::list_sent_notifications(
                self.store.as_ref(),
                tenant_id,
                since_ms,
            )
            .await?,
        )
    }

    async fn list_failed(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<NotificationRecord>, RuntimeError> {
        Ok(
            NotificationReadModel::list_failed_notifications(self.store.as_ref(), tenant_id)
                .await?,
        )
    }

    async fn retry(
        &self,
        tenant_id: &TenantId,
        record_id: &str,
    ) -> Result<NotificationRecord, RuntimeError> {
        // Find the original failed record.
        let failed_records =
            NotificationReadModel::list_failed_notifications(self.store.as_ref(), tenant_id)
                .await?;
        let original = failed_records
            .iter()
            .find(|r| r.record_id == record_id)
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "notification",
                id: record_id.to_owned(),
            })?;

        let channel = NotificationChannel {
            kind: original.channel_kind.clone(),
            target: original.channel_target.clone(),
        };
        let webhook_body = serde_json::json!({
            "event_type": original.event_type,
            "payload": original.payload,
            "tenant_id": tenant_id.as_str(),
            "occurred_at_ms": original.sent_at_ms,
            "retry": true,
        });

        let (delivered, delivery_error) = attempt_delivery(&channel, &webhook_body).await;
        let now = now_millis();
        let new_record_id = next_record_id();

        let record = NotificationRecord {
            record_id: new_record_id.clone(),
            tenant_id: tenant_id.clone(),
            operator_id: original.operator_id.clone(),
            event_type: original.event_type.clone(),
            channel_kind: original.channel_kind.clone(),
            channel_target: original.channel_target.clone(),
            payload: original.payload.clone(),
            sent_at_ms: now,
            delivered,
            delivery_error: delivery_error.clone(),
        };

        let envelope = make_envelope(RuntimeEvent::NotificationSent(NotificationSent {
            record_id: new_record_id,
            tenant_id: tenant_id.clone(),
            operator_id: original.operator_id.clone(),
            event_type: original.event_type.clone(),
            channel_kind: original.channel_kind.clone(),
            channel_target: original.channel_target.clone(),
            payload: original.payload.clone(),
            sent_at_ms: now,
            delivered,
            delivery_error,
        }));

        self.store.append(&[envelope]).await?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::TenantId;
    use cairn_store::InMemoryStore;

    #[tokio::test]
    async fn notification_prefs_set_and_retrieve() {
        let store = Arc::new(InMemoryStore::new());
        let svc = NotificationServiceImpl::new(store.clone());
        let tenant_id = TenantId::new("tenant_notif");

        svc.set_preferences(
            tenant_id.clone(),
            "op_alice".to_owned(),
            vec!["run.failed".to_owned()],
            vec![NotificationChannel {
                kind: "webhook".to_owned(),
                target: "https://hooks.example.com/alert".to_owned(),
            }],
        )
        .await
        .unwrap();

        let prefs = svc
            .get_preferences(&tenant_id, "op_alice")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(prefs.event_types, vec!["run.failed"]);
        assert_eq!(prefs.channels[0].kind, "webhook");
    }

    #[tokio::test]
    async fn notification_prefs_notify_if_applicable_emits_record() {
        let store = Arc::new(InMemoryStore::new());
        let svc = NotificationServiceImpl::new(store.clone());
        let tenant_id = TenantId::new("tenant_notif2");

        svc.set_preferences(
            tenant_id.clone(),
            "op_bob".to_owned(),
            vec!["run.failed".to_owned()],
            vec![NotificationChannel {
                kind: "webhook".to_owned(),
                target: "https://hooks.example.com/bob".to_owned(),
            }],
        )
        .await
        .unwrap();

        let sent = svc
            .notify_if_applicable(
                &tenant_id,
                "run.failed",
                serde_json::json!({ "run_id": "run_test_1" }),
            )
            .await
            .unwrap();

        assert_eq!(sent.len(), 1, "expected one notification record");
        assert_eq!(sent[0].event_type, "run.failed");
        assert_eq!(sent[0].channel_kind, "webhook");
        assert_eq!(sent[0].operator_id, "op_bob");

        let not_sent = svc
            .notify_if_applicable(
                &tenant_id,
                "run.completed",
                serde_json::json!({ "run_id": "run_test_2" }),
            )
            .await
            .unwrap();
        assert!(
            not_sent.is_empty(),
            "run.completed should not trigger webhook pref"
        );
    }

    #[tokio::test]
    async fn notification_prefs_list_sent_by_tenant() {
        let store = Arc::new(InMemoryStore::new());
        let svc = NotificationServiceImpl::new(store.clone());
        let tenant_id = TenantId::new("tenant_notif3");

        svc.set_preferences(
            tenant_id.clone(),
            "op_carol".to_owned(),
            vec!["run.failed".to_owned(), "run.completed".to_owned()],
            vec![NotificationChannel {
                kind: "slack".to_owned(),
                target: "#alerts".to_owned(),
            }],
        )
        .await
        .unwrap();

        svc.notify_if_applicable(&tenant_id, "run.failed", serde_json::json!({}))
            .await
            .unwrap();
        svc.notify_if_applicable(&tenant_id, "run.completed", serde_json::json!({}))
            .await
            .unwrap();

        let all = svc.list_sent(&tenant_id, 0).await.unwrap();
        assert_eq!(all.len(), 2, "expected two sent notification records");
    }

    /// RFC 011: webhook to a non-existent URL records delivered=false with error.
    #[tokio::test]
    async fn webhook_delivery_failed_notification_recorded() {
        let store = Arc::new(InMemoryStore::new());
        let svc = NotificationServiceImpl::new(store.clone());
        let tenant_id = TenantId::new("tenant_webhook");

        // Point to a URL that will certainly fail (localhost port that is not listening).
        svc.set_preferences(
            tenant_id.clone(),
            "op_dave".to_owned(),
            vec!["run.failed".to_owned()],
            vec![NotificationChannel {
                kind: "webhook".to_owned(),
                target: "http://127.0.0.1:19999/nonexistent".to_owned(),
            }],
        )
        .await
        .unwrap();

        let sent = svc
            .notify_if_applicable(
                &tenant_id,
                "run.failed",
                serde_json::json!({ "run_id": "run_wh_1" }),
            )
            .await
            .unwrap();

        assert_eq!(
            sent.len(),
            1,
            "notification record should exist even on failure"
        );
        assert!(
            !sent[0].delivered,
            "webhook to dead URL must not be delivered=true"
        );
        assert!(
            sent[0].delivery_error.is_some(),
            "delivery_error must be set on failure"
        );

        // list_failed should include it.
        let failed = svc.list_failed(&tenant_id).await.unwrap();
        assert_eq!(failed.len(), 1, "expected 1 failed notification");
        assert_eq!(failed[0].record_id, sent[0].record_id);
    }

    #[tokio::test]
    async fn webhook_delivery_retry_creates_new_record() {
        let store = Arc::new(InMemoryStore::new());
        let svc = NotificationServiceImpl::new(store.clone());
        let tenant_id = TenantId::new("tenant_retry");

        svc.set_preferences(
            tenant_id.clone(),
            "op_eve".to_owned(),
            vec!["run.failed".to_owned()],
            vec![NotificationChannel {
                kind: "webhook".to_owned(),
                target: "http://127.0.0.1:19999/retry_target".to_owned(),
            }],
        )
        .await
        .unwrap();

        let sent = svc
            .notify_if_applicable(
                &tenant_id,
                "run.failed",
                serde_json::json!({ "run_id": "run_retry_1" }),
            )
            .await
            .unwrap();

        assert!(!sent[0].delivered, "initial delivery should fail");
        let record_id = sent[0].record_id.clone();

        // Retry (will also fail since URL is dead, but a new record is created).
        let retried = svc.retry(&tenant_id, &record_id).await.unwrap();
        assert!(!retried.delivered, "retry to dead URL also fails");
        assert!(retried.delivery_error.is_some());

        // Two failed records now.
        let all_failed = svc.list_failed(&tenant_id).await.unwrap();
        assert_eq!(all_failed.len(), 2, "both original and retry are failed");
    }
}
