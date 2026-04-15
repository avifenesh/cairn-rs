//! Event replay and append handlers (RFC 002).

#[allow(unused_imports)]
use crate::*;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use cairn_store::{EventLog, EventPosition};
use serde::{Deserialize, Serialize};

// ── Event replay handler (RFC 002) ────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct EventReplayQuery {
    /// Return events strictly after this log position.
    after: Option<u64>,
    #[serde(default = "default_event_limit")]
    limit: usize,
}

pub(crate) fn default_event_limit() -> usize {
    100
}

#[derive(Serialize)]
pub(crate) struct StoredEventSummary {
    position: u64,
    stored_at: u64,
    event_type: String,
}

// Run tool invocations handler → bin_handlers.rs

/// `GET /v1/events` — cursor-based replay of the global event log (RFC 002).
///
/// Clients use `?after=<position>&limit=<n>` to page forward. Returns at most
/// `limit` events (default 100, max 500) strictly after the given position.
/// When Postgres is configured, replays from the durable Postgres log.
pub(crate) async fn list_events_handler(
    State(state): State<AppState>,
    Query(q): Query<EventReplayQuery>,
) -> impl axum::response::IntoResponse {
    let limit = q.limit.min(500);
    let after = q.after.map(EventPosition);
    // Use durable event log for replay when available (Postgres > SQLite > InMemory).
    let read_result = if let Some(pg) = &state.pg {
        pg.event_log.read_stream(after, limit).await
    } else if let Some(sq) = &state.sqlite {
        sq.event_log.read_stream(after, limit).await
    } else {
        state.runtime.store.read_stream(after, limit).await
    };
    match read_result {
        Ok(events) => {
            let summaries: Vec<StoredEventSummary> = events
                .into_iter()
                .map(|e| StoredEventSummary {
                    position: e.position.0,
                    stored_at: e.stored_at,
                    event_type: event_type_name(&e.envelope.payload).to_owned(),
                })
                .collect();
            Ok(Json(summaries))
        }
        Err(e) => Err(internal_error(e.to_string())),
    }
}

// event_type_name is the canonical copy from the lib crate (helpers.rs).
// Re-exported here so binary modules can use it via `use crate::*`.
pub(crate) use cairn_app::event_type_name;

// ── Event append handler (RFC 002) ────────────────────────────────────────────

/// Per-envelope result returned by `POST /v1/events/append`.
#[derive(Serialize)]
pub(crate) struct AppendResult {
    event_id: String,
    position: u64,
    /// `true` = event was newly appended; `false` = idempotent duplicate
    /// (causation_id already existed — existing position is returned).
    appended: bool,
}

/// `POST /v1/events/append` — write path for the event log (RFC 002).
///
/// Accepts a JSON array of `EventEnvelope<RuntimeEvent>` objects. Each
/// envelope is processed for idempotency:
///
/// - If the envelope carries a `causation_id` **and** an event with that
///   causation ID already exists in the log, the existing position is
///   returned without re-appending.
/// - Otherwise the event is appended and its assigned position is returned.
///
/// Appended events are broadcast immediately to all SSE subscribers.
///
/// Returns an array of `AppendResult` in the same order as the input.
pub(crate) async fn append_events_handler(
    State(state): State<AppState>,
    Json(envelopes): Json<Vec<cairn_domain::EventEnvelope<cairn_domain::RuntimeEvent>>>,
) -> impl axum::response::IntoResponse {
    if envelopes.is_empty() {
        return Ok((StatusCode::OK, Json(Vec::<AppendResult>::new())));
    }

    let mut results: Vec<AppendResult> = Vec::with_capacity(envelopes.len());

    for envelope in envelopes {
        let event_id = envelope.event_id.as_str().to_owned();

        // ── Notification hook ──────────────────────────────────────────────────
        // Inspect each event and push a notification for operator-relevant ones.
        {
            use cairn_domain::lifecycle::RunState;
            use cairn_domain::RuntimeEvent as E;
            use std::time::SystemTime;
            let now_ms = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let notif_id = format!("notif-{}", &event_id[..event_id.len().min(16)]);

            let maybe_notif: Option<Notification> = match &envelope.payload {
                E::ApprovalRequested(e) => Some(Notification {
                    id: notif_id,
                    notif_type: NotifType::ApprovalRequested,
                    message: format!(
                        "Approval requested for {}",
                        e.run_id.as_ref().map(|r| r.as_str()).unwrap_or("a task"),
                    ),
                    entity_id: Some(e.approval_id.as_str().to_owned()),
                    href: "approvals".to_owned(),
                    read: false,
                    created_at: now_ms,
                }),
                E::ApprovalResolved(e) => Some(Notification {
                    id: notif_id,
                    notif_type: NotifType::ApprovalResolved,
                    message: format!(
                        "Approval {} — decision: {:?}",
                        e.approval_id.as_str(),
                        e.decision,
                    ),
                    entity_id: Some(e.approval_id.as_str().to_owned()),
                    href: "approvals".to_owned(),
                    read: false,
                    created_at: now_ms,
                }),
                E::RunStateChanged(e) => match &e.transition.to {
                    RunState::Completed => Some(Notification {
                        id: notif_id,
                        notif_type: NotifType::RunCompleted,
                        message: format!("Run {} completed", e.run_id.as_str()),
                        entity_id: Some(e.run_id.as_str().to_owned()),
                        href: format!("run/{}", e.run_id.as_str()),
                        read: false,
                        created_at: now_ms,
                    }),
                    RunState::Failed => Some(Notification {
                        id: notif_id,
                        notif_type: NotifType::RunFailed,
                        message: format!(
                            "Run {} failed{}",
                            e.run_id.as_str(),
                            e.failure_class
                                .as_ref()
                                .map(|f| format!(" ({f:?})"))
                                .unwrap_or_default(),
                        ),
                        entity_id: Some(e.run_id.as_str().to_owned()),
                        href: format!("run/{}", e.run_id.as_str()),
                        read: false,
                        created_at: now_ms,
                    }),
                    _ => None,
                },
                E::TaskStateChanged(e) => {
                    use cairn_domain::lifecycle::TaskState;
                    match &e.transition.to {
                        TaskState::DeadLettered | TaskState::RetryableFailed => {
                            Some(Notification {
                                id: notif_id,
                                notif_type: NotifType::TaskStuck,
                                message: format!(
                                    "Task {} is stuck ({:?})",
                                    e.task_id.as_str(),
                                    e.transition.to,
                                ),
                                entity_id: Some(e.task_id.as_str().to_owned()),
                                href: "tasks".to_owned(),
                                read: false,
                                created_at: now_ms,
                            })
                        }
                        _ => None,
                    }
                }
                _ => None,
            };

            if let Some(n) = maybe_notif {
                if let Ok(mut buf) = state.notifications.write() {
                    buf.push(n);
                }
            }
        }
        // ── End notification hook ──────────────────────────────────────────────

        // Idempotency check: if causation_id is set and already in the log,
        // return the existing position instead of appending.
        if let Some(ref cid) = envelope.causation_id {
            // Check InMemory first (fastest path); Pg check follows when configured.
            let existing = state.runtime.store.find_by_causation_id(cid.as_str()).await;
            match existing {
                Ok(Some(pos)) => {
                    results.push(AppendResult {
                        event_id,
                        position: pos.0,
                        appended: false,
                    });
                    continue;
                }
                Ok(None) => {} // not found — fall through to append
                Err(e) => return Err(internal_error(e.to_string())),
            }
        }

        // Append the single event.
        // Dual-write: persist to durable backend first, then update InMemory
        // so projections and SSE broadcasts stay current.
        if let Some(ref pg) = state.pg {
            if let Err(e) = pg.event_log.append(std::slice::from_ref(&envelope)).await {
                return Err(internal_error(format!("postgres append: {e}")));
            }
        } else if let Some(ref sq) = state.sqlite {
            if let Err(e) = sq.event_log.append(std::slice::from_ref(&envelope)).await {
                return Err(internal_error(format!("sqlite append: {e}")));
            }
        }
        // Always write to InMemory: updates projections + broadcasts to SSE subscribers.
        match state.runtime.store.append(&[envelope]).await {
            Ok(positions) => {
                results.push(AppendResult {
                    event_id,
                    position: positions[0].0,
                    appended: true,
                });
            }
            Err(e) => return Err(internal_error(e.to_string())),
        }
    }

    Ok((StatusCode::CREATED, Json(results)))
}
