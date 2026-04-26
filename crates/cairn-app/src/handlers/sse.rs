//! SSE stream handler and supporting helpers.
//!
//! Extracted from `lib.rs` — contains the runtime SSE stream endpoint,
//! event head helper, frame publishing, and SSE frame construction.

use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::State,
    http::HeaderMap,
    response::sse::{Event as SseEvent, Sse},
};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

use cairn_api::sse::SseFrame;
use cairn_graph::event_projector::EventProjector as RuntimeGraphProjector;
use cairn_store::{EventLog, EventPosition};

use crate::state::AppState;

pub(crate) const SSE_BUFFER_CAPACITY: usize = 10_000;

// ── SSE stream handler ─────────────────────────────────────────────────────

pub(crate) async fn runtime_stream_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(principal): axum::extract::Extension<cairn_api::auth::AuthPrincipal>,
    headers: HeaderMap,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    // T6c-C1: fan out only frames whose tenant matches the subscriber's
    // authenticated tenant (or unscoped frames, e.g. `ready`). Admin
    // principals see every tenant — same semantics as the REST API.
    let subscriber_tenant: Option<String> =
        principal.tenant().map(|t| t.tenant_id.as_str().to_owned());
    let is_admin = crate::extractors::is_admin_principal(&principal);

    // Subscribe to the live broadcast BEFORE reading the replay window so no
    // frames can be missed in the gap between replay and live subscription.
    let receiver = state.runtime_sse_tx.subscribe();

    // Parse Last-Event-ID — the client sends this on reconnect to resume
    // from where it left off (RFC 002 §4).
    let last_seq: Option<u64> = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    // Collect all buffered frames after last_seq, filtered by tenant.
    let replay_frames: Vec<SseFrame> = {
        let buf = state
            .sse_event_buffer
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match last_seq {
            None => vec![],
            Some(after) => buf
                .iter()
                .filter(|(seq, frame)| {
                    *seq > after && frame_visible_to(frame, subscriber_tenant.as_deref(), is_admin)
                })
                .map(|(_, frame)| frame.clone())
                .collect(),
        }
    };

    // Replay stream: historical frames the client missed.
    let replay = tokio_stream::iter(replay_frames)
        .map(|frame| Ok::<SseEvent, Infallible>(sse_event_from_frame(frame)));

    // Live stream: new frames arriving via broadcast, tenant-filtered.
    let subscriber_tenant_live = subscriber_tenant.clone();
    let live = BroadcastStream::new(receiver).filter_map(move |message| match message {
        Ok(frame) => {
            if frame_visible_to(&frame, subscriber_tenant_live.as_deref(), is_admin) {
                Some(Ok(sse_event_from_frame(frame)))
            } else {
                None
            }
        }
        Err(_) => None, // lagged receiver — client will reconnect
    });

    // Replay missed events first, then switch to the live stream.
    let stream = replay.chain(live);
    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

/// T6c-C1: a frame is visible to a subscriber when (a) the caller is
/// an admin, (b) the frame is tenant-agnostic (None), or (c) the
/// frame's tenant matches the subscriber's tenant.
fn frame_visible_to(frame: &SseFrame, subscriber_tenant: Option<&str>, is_admin: bool) -> bool {
    if is_admin {
        return true;
    }
    match (frame.tenant_id.as_deref(), subscriber_tenant) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(ft), Some(st)) => ft == st,
    }
}

pub(crate) fn sse_event_from_frame(frame: SseFrame) -> SseEvent {
    let mut event = SseEvent::default()
        .event(frame.event.as_str())
        .data(serde_json::to_string(&frame.data).unwrap_or_else(|_| "{}".to_owned()));
    if let Some(id) = frame.id {
        event = event.id(id);
    }
    event
}

// ── Helpers ─────────────────────────────────────────────────────────────────

pub(crate) async fn current_event_head(state: &Arc<AppState>) -> Option<EventPosition> {
    state.runtime.store.head_position().await.ok().flatten()
}

pub(crate) async fn publish_runtime_frames_since(
    state: &Arc<AppState>,
    after: Option<EventPosition>,
) {
    let Ok(events) = state.runtime.store.read_stream(after, 64).await else {
        return;
    };

    let projector = RuntimeGraphProjector::new(state.graph.clone());
    let _ = projector.project_events(&events).await;

    for stored in events {
        // Invalidate the provider_registry cache on any provider-connection
        // mutation so subsequent routes don't hit a stale entry. F40: the
        // old DELETE path masqueraded as a `Registered` event so a single
        // match arm sufficed; the new `Deleted` variant must invalidate too.
        match &stored.envelope.payload {
            cairn_domain::RuntimeEvent::ProviderConnectionRegistered(connection) => {
                state
                    .runtime
                    .provider_registry
                    .invalidate(&connection.provider_connection_id);
            }
            cairn_domain::RuntimeEvent::ProviderConnectionDeleted(connection) => {
                state
                    .runtime
                    .provider_registry
                    .invalidate(&connection.provider_connection_id);
            }
            _ => {}
        }

        // F50: push a notification on operator-visible events. Until now
        // the hook only fired on `/v1/events/append` (admin write path);
        // service-layer appends (approval requested/resolved, run
        // completed/failed, task stuck) bypassed it so the bell icon +
        // sidebar badge stayed empty. Running this inside the same
        // publish loop that already drives the SSE stream guarantees
        // parity: every event that reaches the SSE frame also reaches
        // the notification buffer. Notification id + created_at are
        // derived from the envelope (event_id + stored_at) so replay
        // of the same event produces an identical notification — no
        // wall-clock-based duplicates.
        push_notification_for_event(state, &stored);

        // F49: auto-resume orchestrate kick. When an approval resolves,
        // check whether any other pending approvals block the run. If
        // none AND the run is still Running, enqueue a kick for the
        // background worker to POST /v1/runs/:id/orchestrate again so
        // the operator doesn't have to.
        if let cairn_domain::RuntimeEvent::ApprovalResolved(e) = &stored.envelope.payload {
            maybe_auto_resume_orchestrate(state, e).await;
        }

        // OTLP export (RFC 021): send each event to the exporter.
        let _ = state
            .otlp_exporter
            .export_event(&stored.envelope.payload)
            .await;

        if let Some(mut frame) = build_runtime_sse_frame(state, &stored).await {
            // Assign a monotonic sequence ID for Last-Event-ID replay.
            let seq = state
                .sse_seq
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            frame.id = Some(seq.to_string());

            // Push to replay buffer (trim oldest if at capacity).
            {
                let mut buf = state
                    .sse_event_buffer
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                if buf.len() >= SSE_BUFFER_CAPACITY {
                    buf.pop_front();
                }
                buf.push_back((seq, frame.clone()));
            }

            let _ = state.runtime_sse_tx.send(frame);
        }
    }
}

pub(crate) async fn build_runtime_sse_frame(
    state: &Arc<AppState>,
    stored: &cairn_store::StoredEvent,
) -> Option<SseFrame> {
    let mut frame = cairn_api::sse_publisher::build_sse_frame_with_store_state(
        state.runtime.store.as_ref(),
        stored,
    )
    .await
    .ok()
    .flatten()?;

    if let Some(correlation_id) = stored.envelope.correlation_id.as_ref() {
        match &mut frame.data {
            serde_json::Value::Object(map) => {
                map.insert(
                    "correlation_id".to_owned(),
                    serde_json::Value::String(correlation_id.clone()),
                );
            }
            payload => {
                frame.data = serde_json::json!({
                    "payload": payload.clone(),
                    "correlation_id": correlation_id,
                });
            }
        }
    }

    // T6c-C1: tag the frame with the originating envelope's tenant so
    // the SSE handler can filter fan-out by the subscriber's principal.
    // Use the canonical ownership field rather than re-walking variants
    // (Gemini R1: `EventEnvelope::ownership` is the definitive source).
    frame.tenant_id = ws_event_tenant_id(&stored.envelope).map(|s| s.to_owned());

    Some(frame)
}

/// T6c-C2 (+ R1): tenant-id helper shared by the SSE publisher and the
/// WebSocket fan-out. Reads `EventEnvelope::ownership` — the canonical
/// tenancy field on every event — so it stays correct as new
/// `RuntimeEvent` variants are added. `System`-owned envelopes return
/// `None` (tenant-agnostic; visible to everyone, same as `ready`).
pub fn ws_event_tenant_id(
    envelope: &cairn_domain::EventEnvelope<cairn_domain::RuntimeEvent>,
) -> Option<&str> {
    match &envelope.ownership {
        cairn_domain::tenancy::OwnershipKey::System => None,
        cairn_domain::tenancy::OwnershipKey::Tenant(k) => Some(k.tenant_id.as_str()),
        cairn_domain::tenancy::OwnershipKey::Workspace(k) => Some(k.tenant_id.as_str()),
        cairn_domain::tenancy::OwnershipKey::Project(k) => Some(k.tenant_id.as_str()),
    }
}

// ── F50: notification projection hook ────────────────────────────────────────

/// F50: push a `Notification` into the in-memory buffer whenever an
/// operator-relevant event is published. Mirrors the existing hook
/// inside `bin_events.rs` (which only fires on admin `POST /v1/events/
/// append` and therefore missed service-layer writes). Kept here so
/// every store append routed through `publish_runtime_frames_since`
/// surfaces in the bell icon + sidebar badge.
fn push_notification_for_event(state: &Arc<AppState>, stored: &cairn_store::StoredEvent) {
    use crate::state::{OperatorNotification, OperatorNotificationType as T};
    use cairn_domain::lifecycle::{RunState, TaskState};
    use cairn_domain::RuntimeEvent as E;

    // Derived from the envelope rather than `SystemTime::now()` so the
    // same stored event always produces the same notification id +
    // created_at. Matches the admin `/v1/events/append` hook's
    // `notif-<event_id_prefix>` strategy and keeps the bell buffer
    // idempotent under SSE reconnect replay — no duplicates from the
    // wall-clock stamp drifting across ticks.
    let event_id = stored.envelope.event_id.as_str();
    let created_at_ms = stored.stored_at;
    let trunc: String = event_id.chars().take(16).collect();
    let make_id = |kind: &str| format!("notif-{kind}-{trunc}");
    let payload = &stored.envelope.payload;
    let tenant_id = ws_event_tenant_id(&stored.envelope).map(|s| s.to_owned());

    let maybe_notif: Option<OperatorNotification> = match payload {
        E::ApprovalRequested(e) => Some(OperatorNotification {
            id: make_id("appreq"),
            notif_type: T::ApprovalRequested,
            message: format!(
                "Approval requested for {}",
                e.run_id.as_ref().map(|r| r.as_str()).unwrap_or("a task"),
            ),
            entity_id: Some(e.approval_id.as_str().to_owned()),
            href: "approvals".to_owned(),
            created_at_ms,
            tenant_id: tenant_id.clone(),
        }),
        E::ApprovalResolved(e) => Some(OperatorNotification {
            id: make_id("appres"),
            notif_type: T::ApprovalResolved,
            message: format!(
                "Approval {} — decision: {:?}",
                e.approval_id.as_str(),
                e.decision,
            ),
            entity_id: Some(e.approval_id.as_str().to_owned()),
            href: "approvals".to_owned(),
            created_at_ms,
            tenant_id: tenant_id.clone(),
        }),
        E::RunStateChanged(e) => match &e.transition.to {
            RunState::Completed => Some(OperatorNotification {
                id: make_id("runok"),
                notif_type: T::RunCompleted,
                message: format!("Run {} completed", e.run_id.as_str()),
                entity_id: Some(e.run_id.as_str().to_owned()),
                href: format!("run/{}", e.run_id.as_str()),
                created_at_ms,
                tenant_id: tenant_id.clone(),
            }),
            RunState::Failed => Some(OperatorNotification {
                id: make_id("runfail"),
                notif_type: T::RunFailed,
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
                created_at_ms,
                tenant_id: tenant_id.clone(),
            }),
            _ => None,
        },
        E::TaskStateChanged(e) => match &e.transition.to {
            TaskState::DeadLettered | TaskState::RetryableFailed => Some(OperatorNotification {
                id: make_id("stuck"),
                notif_type: T::TaskStuck,
                message: format!(
                    "Task {} is stuck ({:?})",
                    e.task_id.as_str(),
                    e.transition.to,
                ),
                entity_id: Some(e.task_id.as_str().to_owned()),
                href: "tasks".to_owned(),
                created_at_ms,
                tenant_id: tenant_id.clone(),
            }),
            _ => None,
        },
        _ => None,
    };

    if let Some(n) = maybe_notif {
        state.notification_sink.push(n);
    }
}

// ── F49: auto-resume orchestrate kick ────────────────────────────────────────

/// F49: if the resolved approval belongs to a run and no other pending
/// approvals exist for the same run and the run is still in state
/// `Running`, enqueue a kick on the `orchestrate_kick_tx` channel so
/// the background worker re-enters the orchestrate loop without
/// operator action.
///
/// Every check is best-effort — a transient store blip or a missing
/// run_id degrades gracefully to the pre-F49 behaviour (operator
/// re-POSTs `/orchestrate`). No panics, no 5xxs from this path.
async fn maybe_auto_resume_orchestrate(state: &Arc<AppState>, e: &cairn_domain::ApprovalResolved) {
    use cairn_store::projections::{ApprovalReadModel, RunReadModel};
    let Some(run_id) = resolve_run_for_approval(state, &e.approval_id).await else {
        return;
    };

    // Any other pending approvals for this run block auto-resume.
    match ApprovalReadModel::has_pending_for_run(state.runtime.store.as_ref(), &run_id).await {
        Ok(true) => return,
        Ok(false) => {}
        Err(err) => {
            tracing::debug!(
                run_id = %run_id,
                error = %err,
                "F49: skip auto-resume (cannot read pending approvals)"
            );
            return;
        }
    }

    // Run must exist and still be Running — auto-resume on a terminal
    // or paused run would race with the state machine.
    let run_rec = match RunReadModel::get(state.runtime.store.as_ref(), &run_id).await {
        Ok(Some(r)) => r,
        _ => return,
    };
    if run_rec.state != cairn_domain::RunState::Running {
        return;
    }

    // Best-effort enqueue. `kick` returns false when the channel is
    // not installed yet (early startup) — same behaviour as pre-F49.
    if state.orchestrate_kick_tx.kick(run_id.clone()) {
        tracing::info!(
            run_id = %run_id,
            approval_id = %e.approval_id,
            "F49: queued auto-resume orchestrate kick"
        );
    }
}

/// Walk from approval_id → run_id via the approval projection. The
/// `ApprovalResolved` frame only carries the id, so we must look up
/// the run association in the projection.
///
/// Transient store errors are logged at debug and degrade to "no
/// run_id" so auto-resume is best-effort — operators can always
/// re-POST `/orchestrate` manually (the pre-F49 behaviour).
async fn resolve_run_for_approval(
    state: &Arc<AppState>,
    approval_id: &cairn_domain::ApprovalId,
) -> Option<cairn_domain::RunId> {
    use cairn_store::projections::ApprovalReadModel;
    match state.runtime.store.get(approval_id).await {
        Ok(Some(rec)) => rec.run_id,
        Ok(None) => None,
        Err(err) => {
            tracing::debug!(
                approval_id = %approval_id,
                error = %err,
                "F49: skip auto-resume (cannot read approval record from projection)"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_api::sse::{SseEventName, SseFrame};

    fn frame(tenant_id: Option<&str>) -> SseFrame {
        SseFrame {
            event: SseEventName::Ready,
            data: serde_json::Value::Null,
            id: None,
            tenant_id: tenant_id.map(|s| s.to_owned()),
        }
    }

    #[test]
    fn admin_sees_every_frame_regardless_of_tenant() {
        assert!(frame_visible_to(&frame(Some("acme")), None, true));
        assert!(frame_visible_to(&frame(Some("globex")), Some("acme"), true));
        assert!(frame_visible_to(&frame(None), None, true));
    }

    #[test]
    fn tenant_agnostic_frames_broadcast_to_all_subscribers() {
        assert!(frame_visible_to(&frame(None), Some("acme"), false));
        assert!(frame_visible_to(&frame(None), None, false));
    }

    #[test]
    fn tenant_scoped_frame_refused_when_subscriber_has_no_tenant() {
        // Gemini R1: the prior `if let (Some, Some)` implementation leaked
        // tenant-scoped frames to unscoped callers. Subscribers without a
        // tenant should never see tenant-scoped frames.
        assert!(!frame_visible_to(&frame(Some("acme")), None, false));
    }

    #[test]
    fn tenant_scoped_frame_refused_on_mismatch() {
        assert!(!frame_visible_to(
            &frame(Some("acme")),
            Some("globex"),
            false
        ));
    }

    #[test]
    fn tenant_scoped_frame_delivered_on_match() {
        assert!(frame_visible_to(&frame(Some("acme")), Some("acme"), false));
    }

    #[test]
    fn ws_event_tenant_id_reads_ownership_field() {
        use cairn_domain::ids::{EventId, SessionId};
        use cairn_domain::tenancy::{OwnershipKey, ProjectKey, TenantKey, WorkspaceKey};

        // Use SessionCreated as a concrete, minimal RuntimeEvent payload;
        // the helper cares about `ownership`, not the payload variant.
        let payload = cairn_domain::RuntimeEvent::SessionCreated(cairn_domain::SessionCreated {
            project: ProjectKey::new("unused", "unused", "unused"),
            session_id: SessionId::new("s1"),
        });

        let envelope = |ownership: OwnershipKey| {
            cairn_domain::EventEnvelope::new(
                EventId::new("evt"),
                cairn_domain::EventSource::System,
                ownership,
                payload.clone(),
            )
        };

        assert_eq!(ws_event_tenant_id(&envelope(OwnershipKey::System)), None);
        assert_eq!(
            ws_event_tenant_id(&envelope(OwnershipKey::Project(ProjectKey::new(
                "t1", "w1", "p1"
            )))),
            Some("t1"),
        );
        assert_eq!(
            ws_event_tenant_id(&envelope(OwnershipKey::Workspace(WorkspaceKey::new(
                "t2", "w2"
            )))),
            Some("t2"),
        );
        assert_eq!(
            ws_event_tenant_id(&envelope(OwnershipKey::Tenant(TenantKey::new("t3")))),
            Some("t3"),
        );
    }
}
