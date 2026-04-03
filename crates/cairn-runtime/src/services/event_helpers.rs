use cairn_domain::*;
use std::sync::atomic::{AtomicU64, Ordering};

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn next_event_id() -> EventId {
    let n = EVENT_COUNTER.fetch_add(1, Ordering::SeqCst);
    EventId::new(format!("evt_{n}"))
}

pub fn make_envelope(payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(next_event_id(), EventSource::Runtime, payload)
}
