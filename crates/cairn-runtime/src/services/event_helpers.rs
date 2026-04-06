use cairn_domain::*;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::get_current_trace_id;

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn next_event_id() -> EventId {
    let n = EVENT_COUNTER.fetch_add(1, Ordering::SeqCst);
    EventId::new(format!("evt_{n}"))
}

pub fn make_envelope(payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    let mut envelope =
        EventEnvelope::for_runtime_event(next_event_id(), EventSource::Runtime, payload);
    let trace_id = get_current_trace_id();
    if !trace_id.is_empty() {
        envelope = envelope.with_correlation_id(trace_id);
    }
    envelope
}
