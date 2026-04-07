use cairn_domain::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::get_current_trace_id;

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Initialise the event counter to start above `floor`.
///
/// Call this once at startup (after startup replay) to ensure that
/// service-generated event IDs never collide with events already persisted
/// in a durable store.  The counter is advanced to `max(current, floor + 1)`.
pub fn seed_event_counter(floor: u64) {
    let target = floor + 1;
    let mut current = EVENT_COUNTER.load(Ordering::SeqCst);
    loop {
        if current >= target {
            break;
        }
        match EVENT_COUNTER.compare_exchange(current, target, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

pub fn next_event_id() -> EventId {
    let n = EVENT_COUNTER.fetch_add(1, Ordering::SeqCst);
    // Include a timestamp prefix so event IDs are globally unique across
    // process restarts even if seed_event_counter was not called.
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    EventId::new(format!("evt_{ts_ms}_{n}"))
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
