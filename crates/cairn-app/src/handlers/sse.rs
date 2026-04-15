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
    headers: HeaderMap,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    // Subscribe to the live broadcast BEFORE reading the replay window so no
    // frames can be missed in the gap between replay and live subscription.
    let receiver = state.runtime_sse_tx.subscribe();

    // Parse Last-Event-ID — the client sends this on reconnect to resume
    // from where it left off (RFC 002 §4).
    let last_seq: Option<u64> = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    // Collect all buffered frames after last_seq.
    let replay_frames: Vec<SseFrame> = {
        let buf = state
            .sse_event_buffer
            .read()
            .expect("sse_event_buffer poisoned");
        match last_seq {
            None => vec![],
            Some(after) => buf
                .iter()
                .filter(|(seq, _)| *seq > after)
                .map(|(_, frame)| frame.clone())
                .collect(),
        }
    };

    // Replay stream: historical frames the client missed.
    let replay = tokio_stream::iter(replay_frames)
        .map(|frame| Ok::<SseEvent, Infallible>(sse_event_from_frame(frame)));

    // Live stream: new frames arriving via broadcast.
    let live = BroadcastStream::new(receiver).filter_map(|message| match message {
        Ok(frame) => Some(Ok(sse_event_from_frame(frame))),
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
                    .expect("sse_event_buffer poisoned");
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

    Some(frame)
}
