use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{RunReadModel, SignalReadModel, SignalSubscriptionReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::signal_routing::{SignalRouterService, SignalRoutingResult};

pub struct SignalRouterServiceImpl<S> {
    store: Arc<S>,
}

impl<S> SignalRouterServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn signal_matches_filter(record: &SignalRecord, filter_expression: Option<&str>) -> bool {
    filter_expression.is_none_or(|expr| record.payload.to_string().contains(expr))
}

#[async_trait]
impl<S> SignalRouterService for SignalRouterServiceImpl<S>
where
    S: EventLog
        + SignalReadModel
        + SignalSubscriptionReadModel
        + RunReadModel
        + Send
        + Sync
        + 'static,
{
    async fn subscribe(
        &self,
        project: ProjectKey,
        signal_kind: String,
        target_run_id: Option<RunId>,
        target_mailbox_id: Option<String>,
        filter_expression: Option<String>,
    ) -> Result<SignalSubscription, RuntimeError> {
        if target_run_id.is_none() && target_mailbox_id.is_none() {
            return Err(RuntimeError::Internal(
                "signal subscription requires a run or mailbox target".to_owned(),
            ));
        }

        if let Some(run_id) = &target_run_id {
            let run = RunReadModel::get(self.store.as_ref(), run_id).await?;
            if run.is_none() {
                return Err(RuntimeError::NotFound {
                    entity: "run",
                    id: run_id.to_string(),
                });
            }
        }

        let now = now_ms();
        let subscription = SignalSubscription {
            subscription_id: format!("signal_sub_{now}"),
            project: project.clone(),
            signal_kind: signal_kind.clone(),
            target_run_id: target_run_id.clone(),
            target_mailbox_id: target_mailbox_id.clone(),
            filter_expression: filter_expression.clone(),
            created_at_ms: now,
            signal_type: signal_kind.clone(),
            target: target_run_id
                .as_ref()
                .map(|id| id.as_str().to_owned())
                .unwrap_or_default(),
        };

        self.store
            .append(&[make_envelope(RuntimeEvent::SignalSubscriptionCreated(
                SignalSubscriptionCreated {
                    project,
                    subscription_id: subscription.subscription_id.clone(),
                    signal_kind,
                    target_run_id,
                    target_mailbox_id,
                    filter_expression,
                    created_at_ms: now,
                },
            ))])
            .await?;

        Ok(subscription)
    }

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SignalSubscription>, RuntimeError> {
        let records = SignalSubscriptionReadModel::list_by_project(
            self.store.as_ref(),
            project,
            limit,
            offset,
        )
        .await?;
        Ok(records
            .into_iter()
            .map(|rec| SignalSubscription {
                subscription_id: rec.subscription_id,
                signal_type: rec.signal_type.clone(),
                target: rec.target,
                created_at_ms: rec.created_at_ms,
                project: rec.project.unwrap_or_else(|| project.clone()),
                signal_kind: rec.signal_type,
                target_run_id: rec.target_run_id,
                target_mailbox_id: rec.target_mailbox_id,
                filter_expression: rec.filter_expression,
            })
            .collect())
    }

    async fn route_signal(
        &self,
        signal_id: &SignalId,
    ) -> Result<SignalRoutingResult, RuntimeError> {
        let signal = SignalReadModel::get(self.store.as_ref(), signal_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "signal",
                id: signal_id.to_string(),
            })?;

        // Signals currently route on their source field, which is the canonical kind on ingest.
        let subscriptions = SignalSubscriptionReadModel::list_by_signal_kind(
            self.store.as_ref(),
            signal.source.as_str(),
            usize::MAX,
            0,
        )
        .await?;

        let mut events = Vec::new();
        let mut mailbox_message_ids = Vec::new();
        let delivered_at_ms = now_ms();

        for subscription in subscriptions {
            let sub_project = subscription
                .project
                .clone()
                .unwrap_or_else(|| signal.project.clone());
            if sub_project != signal.project
                || !signal_matches_filter(&signal, subscription.filter_expression.as_deref())
            {
                continue;
            }

            if let Some(run_id) = &subscription.target_run_id {
                let run = RunReadModel::get(self.store.as_ref(), run_id).await?;
                if run.is_none() {
                    continue;
                }
            }

            if let Some(target_mailbox_id) = &subscription.target_mailbox_id {
                let message_id = MailboxMessageId::new(target_mailbox_id.as_str());
                mailbox_message_ids.push(message_id.clone());
                events.push(make_envelope(RuntimeEvent::MailboxMessageAppended(
                    MailboxMessageAppended {
                        project: sub_project.clone(),
                        message_id,
                        run_id: subscription.target_run_id.clone(),
                        task_id: None,
                        from_task_id: None,
                        from_run_id: None,
                        content: signal.payload.to_string(),
                        deliver_at_ms: 0,
                        sender: None,
                        recipient: None,
                        body: None,
                        sent_at: None,
                        delivery_status: None,
                    },
                )));
            } else if let Some(run_id) = &subscription.target_run_id {
                let message_id = MailboxMessageId::new(format!(
                    "signal_route_{}_{}",
                    subscription.subscription_id,
                    signal.id.as_str()
                ));
                mailbox_message_ids.push(message_id.clone());
                events.push(make_envelope(RuntimeEvent::MailboxMessageAppended(
                    MailboxMessageAppended {
                        project: sub_project.clone(),
                        message_id,
                        run_id: Some(RunId::new(run_id.as_str())),
                        task_id: None,
                        from_task_id: None,
                        from_run_id: None,
                        content: signal.payload.to_string(),
                        deliver_at_ms: 0,
                        sender: None,
                        recipient: None,
                        body: None,
                        sent_at: None,
                        delivery_status: None,
                    },
                )));
            }

            events.push(make_envelope(RuntimeEvent::SignalRouted(SignalRouted {
                project: sub_project.clone(),
                signal_id: signal.id.clone(),
                subscription_id: subscription.subscription_id.clone(),
                delivered_at_ms,
            })));
        }

        if !events.is_empty() {
            self.store.append(&events).await?;
        }

        Ok(SignalRoutingResult {
            routed_count: mailbox_message_ids.len() as u32,
            mailbox_message_ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::{MailboxMessageId, ProjectKey, RunId, RuntimeEvent, SessionId, SignalId};
    use cairn_store::projections::MailboxReadModel;
    use cairn_store::{EventLog, InMemoryStore};

    use crate::services::{
        RunServiceImpl, SessionServiceImpl, SignalRouterServiceImpl, SignalServiceImpl,
    };
    use crate::{SessionService, SignalRouterService, SignalService};

    fn test_project() -> ProjectKey {
        ProjectKey::new("tenant_acme", "ws_main", "project_alpha")
    }

    #[tokio::test]
    async fn signal_router_routes_alert_signal_to_target_mailbox() {
        let store = Arc::new(InMemoryStore::new());
        let session_service = SessionServiceImpl::new(store.clone());
        let run_service = RunServiceImpl::new(store.clone());
        let signal_service = SignalServiceImpl::new(store.clone());
        let router = SignalRouterServiceImpl::new(store.clone());
        let project = test_project();

        session_service
            .create(&project, SessionId::new("session_signal"))
            .await
            .unwrap();
        run_service
            .start(
                &project,
                &SessionId::new("session_signal"),
                RunId::new("run_signal"),
                None,
            )
            .await
            .unwrap();

        let subscription = router
            .subscribe(
                project.clone(),
                "alert".to_owned(),
                Some(RunId::new("run_signal")),
                Some("mailbox_alert".to_owned()),
                None,
            )
            .await
            .unwrap();

        signal_service
            .ingest(
                &project,
                SignalId::new("sig_alert"),
                "alert".to_owned(),
                serde_json::json!({"severity": "high"}),
                1_000,
            )
            .await
            .unwrap();

        let routed = router
            .route_signal(&SignalId::new("sig_alert"))
            .await
            .unwrap();
        assert_eq!(routed.routed_count, 1);
        assert_eq!(
            routed.mailbox_message_ids,
            vec![MailboxMessageId::new("mailbox_alert")]
        );

        let mailbox =
            MailboxReadModel::get(store.as_ref(), &MailboxMessageId::new("mailbox_alert"))
                .await
                .unwrap();
        assert!(mailbox.is_some(), "signal should be routed into mailbox");

        let events = store.read_stream(None, 20).await.unwrap();
        assert!(events.iter().any(|event| matches!(
            &event.envelope.payload,
            RuntimeEvent::SignalSubscriptionCreated(e) if e.subscription_id == subscription.subscription_id
        )));
        assert!(events.iter().any(|event| matches!(
            &event.envelope.payload,
            RuntimeEvent::SignalRouted(e)
                if e.signal_id == SignalId::new("sig_alert")
                    && e.subscription_id == subscription.subscription_id
        )));
    }
}
