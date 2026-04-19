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
        if let cairn_domain::RuntimeEvent::ProviderConnectionRegistered(connection) =
            &stored.envelope.payload
        {
            state
                .runtime
                .provider_registry
                .invalidate(&connection.provider_connection_id);
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

    // T6c-C1: tag the frame with the originating event's tenant so the
    // SSE handler can filter fan-out by the subscriber's principal.
    frame.tenant_id =
        project_key_from_event(&stored.envelope.payload).map(|pk| pk.tenant_id.as_str().to_owned());

    Some(frame)
}

/// T6c-C2: tenant-id helper for the WebSocket path. Shares the
/// project-key lookup table with the SSE publisher; returns the
/// `tenant_id` string so `bin_websocket::handle_ws_connection` can
/// compare directly against `principal.tenant().tenant_id`.
pub fn ws_event_tenant_id(payload: &cairn_domain::RuntimeEvent) -> Option<&str> {
    project_key_from_event(payload).map(|pk| pk.tenant_id.as_str())
}

/// T6c-C1: extract the ProjectKey from a RuntimeEvent so the SSE
/// publisher can tag the outgoing frame with its tenant. Returns None
/// for tenant-agnostic events (if any); those frames broadcast to
/// every authenticated subscriber.
fn project_key_from_event(
    payload: &cairn_domain::RuntimeEvent,
) -> Option<&cairn_domain::ProjectKey> {
    use cairn_domain::RuntimeEvent::*;
    match payload {
        RunCreated(e) => Some(&e.project),
        RunStateChanged(e) => Some(&e.project),
        TaskCreated(e) => Some(&e.project),
        TaskStateChanged(e) => Some(&e.project),
        TaskLeaseClaimed(e) => Some(&e.project),
        ApprovalRequested(e) => Some(&e.project),
        ApprovalResolved(e) => Some(&e.project),
        SessionCreated(e) => Some(&e.project),
        SessionStateChanged(e) => Some(&e.project),
        MailboxMessageAppended(e) => Some(&e.project),
        // Extensive — the full match is in `StoredEvent`'s ownership
        // field. When new variants are added and don't fit the above,
        // fall through to None (tenant-agnostic) and a CI test catches
        // the divergence.
        _ => None,
    }
}
