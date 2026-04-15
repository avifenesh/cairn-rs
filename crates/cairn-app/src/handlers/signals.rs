//! Signal ingest and subscription handlers.
//!
//! Extracted from `lib.rs` — contains signal ingestion, listing,
//! subscription CRUD, and signal-to-trigger routing.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::http::ListResponse;
use cairn_domain::{ProjectKey, RunId, SignalId};
use cairn_runtime::{SignalRouterService, SignalService};
use cairn_store::EventLog;

use crate::errors::{now_ms, runtime_error_response, AppApiError};
use crate::extractors::OptionalProjectScopedQuery;
use crate::helpers::feed_item_from_signal;
use crate::state::{AppMailboxMessage, AppState};
use crate::triggers::{
    materialize_triggered_run, runtime_event_for_trigger_service_event,
    trigger_decision_outcomes_for_signal, unavailable_trigger_decision, PendingTriggeredRun,
};

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct IngestSignalRequest {
    pub tenant_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub signal_id: String,
    pub source: String,
    pub payload: serde_json::Value,
    pub timestamp_ms: Option<u64>,
}

impl IngestSignalRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateSignalSubscriptionRequest {
    pub tenant_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub signal_kind: String,
    pub target_run_id: Option<String>,
    pub target_mailbox_id: Option<String>,
    pub filter_expression: Option<String>,
}

impl CreateSignalSubscriptionRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

pub(crate) async fn ingest_signal_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<IngestSignalRequest>,
) -> impl IntoResponse {
    let project = body.project();
    let timestamp_ms = body.timestamp_ms.unwrap_or_else(now_ms);
    let before = crate::handlers::sse::current_event_head(&state).await;
    match state
        .runtime
        .signals
        .ingest(
            &project,
            SignalId::new(body.signal_id.clone()),
            body.source.clone(),
            body.payload.clone(),
            timestamp_ms,
        )
        .await
    {
        Ok(record) => {
            state.feed.push_item(feed_item_from_signal(&record));
            // Route signal to subscribers
            if let Ok(routed) = state.runtime.signal_router.route_signal(&record.id).await {
                if !routed.mailbox_message_ids.is_empty() {
                    let mut mailbox_messages = state
                        .mailbox_messages
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    for message_id in routed.mailbox_message_ids {
                        mailbox_messages.insert(
                            message_id.to_string(),
                            AppMailboxMessage {
                                sender_id: Some(format!("signal:{}", record.source)),
                                body: Some(record.payload.to_string()),
                                delivered: true,
                            },
                        );
                    }
                }
            }

            // RFC 022: evaluate triggers for this signal
            let decision_candidates = {
                let triggers = state.triggers.lock().unwrap();
                triggers.decision_candidates_for_signal(
                    &project,
                    &record.id,
                    &record.source,
                    "", // plugin_id — empty for direct API signals
                    &record.payload,
                    None, // source_run_chain_depth
                )
            };
            let trigger_decision_outcomes: HashMap<
                cairn_domain::TriggerId,
                cairn_runtime::services::trigger_service::TriggerDecisionOutcome,
            > = trigger_decision_outcomes_for_signal(
                state.as_ref(),
                &project,
                &record.id,
                &record.source,
                decision_candidates,
            )
            .await;
            let prepared_trigger_ids: HashSet<_> =
                trigger_decision_outcomes.keys().cloned().collect();
            let (pending_runs, persisted_trigger_events) = {
                let mut triggers = state.triggers.lock().unwrap();
                let trigger_events = triggers.evaluate_signal_for_candidates(
                    &project,
                    &record.id,
                    &record.source,
                    "", // plugin_id — empty for direct API signals
                    &record.payload,
                    None, // source_run_chain_depth
                    &prepared_trigger_ids,
                    &|trigger_id, signal_type| {
                        trigger_decision_outcomes.get(trigger_id).cloned().unwrap_or_else(|| {
                            tracing::warn!(
                                project = ?project,
                                trigger_id = %trigger_id,
                                signal_id = %record.id,
                                signal_type,
                                "trigger evaluation reached decision phase without a prepared decision"
                            );
                            unavailable_trigger_decision(
                                trigger_id,
                                format!(
                                    "decision_unavailable_for_trigger_fire:{signal_type}"
                                ),
                            )
                        })
                    },
                );
                let persisted_trigger_events: Vec<cairn_domain::RuntimeEvent> = trigger_events
                    .iter()
                    .filter_map(|event| runtime_event_for_trigger_service_event(&project, event))
                    .collect();
                crate::telemetry_routes::record_trigger_fire_usage(
                    state.runtime.store.as_ref(),
                    &project,
                    &trigger_events,
                );
                let pending_runs: Vec<PendingTriggeredRun> = trigger_events
                    .iter()
                    .filter_map(|event| {
                        let cairn_runtime::services::trigger_service::TriggerEvent::TriggerFired {
                            trigger_id,
                            run_id,
                            ..
                        } = event
                        else {
                            return None;
                        };
                        let Some(trigger) = triggers.get_trigger(trigger_id).cloned() else {
                            tracing::warn!(
                                trigger_id = %trigger_id,
                                "trigger fired but trigger definition was unavailable during materialization"
                            );
                            return None;
                        };
                        let Some(template) =
                            triggers.get_template(&trigger.run_template_id).cloned()
                        else {
                            tracing::warn!(
                                trigger_id = %trigger_id,
                                run_template_id = %trigger.run_template_id,
                                "trigger fired but run template was unavailable during materialization"
                            );
                            return None;
                        };
                        Some(PendingTriggeredRun {
                            trigger_id: trigger_id.clone(),
                            run_id: run_id.clone(),
                            template,
                        })
                    })
                    .collect();
                for event in &trigger_events {
                    if let cairn_runtime::services::trigger_service::TriggerEvent::TriggerFired {
                        trigger_id,
                        run_id,
                        ..
                    } = event
                    {
                        tracing::info!(
                            trigger_id = %trigger_id,
                            run_id = %run_id,
                            "trigger fired — run created from signal"
                        );
                    }
                }
                (pending_runs, persisted_trigger_events)
            };

            if !persisted_trigger_events.is_empty() {
                let envelopes: Vec<_> = persisted_trigger_events
                    .into_iter()
                    .map(cairn_runtime::make_envelope)
                    .collect();
                if let Err(error) = state.runtime.store.append(&envelopes).await {
                    tracing::warn!(
                        project = ?project,
                        error = %error,
                        "failed to persist trigger events during signal ingest"
                    );
                }
            }

            for pending_run in pending_runs {
                if let Err(err) =
                    materialize_triggered_run(state.as_ref(), &project, pending_run).await
                {
                    tracing::warn!(
                        project = ?project,
                        error = %err,
                        "failed to materialize trigger-fired run"
                    );
                }
            }

            crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
            (StatusCode::CREATED, Json(record)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_signals_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .signals
        .list_by_project(&query.project(), query.limit(), query.offset())
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn create_signal_subscription_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSignalSubscriptionRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .signal_router
        .subscribe(
            body.project(),
            body.signal_kind,
            body.target_run_id.map(RunId::new),
            body.target_mailbox_id,
            body.filter_expression,
        )
        .await
    {
        Ok(subscription) => (StatusCode::CREATED, Json(subscription)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_signal_subscriptions_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .signal_router
        .list_by_project(&query.project(), query.limit(), query.offset())
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn delete_signal_subscription_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.runtime.store.delete_signal_subscription(&id).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(err) => {
            tracing::error!("delete_signal_subscription failed: {err}");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response()
        }
    }
}
