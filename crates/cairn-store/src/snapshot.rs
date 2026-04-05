use serde::{Deserialize, Serialize};

use crate::event_log::StoredEvent;

/// Full export of an `InMemoryStore`'s event log.
///
/// The event log is the sole source of truth; all projections are derived
/// by replaying it. Serialising only the events is therefore sufficient
/// to capture and restore the complete store state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoreSnapshot {
    /// Format version — bump if the schema changes incompatibly.
    pub version: u32,
    /// Unix milliseconds when the snapshot was created.
    pub created_at_ms: u64,
    /// Informational: number of events in this snapshot.
    pub event_count: u64,
    /// The raw event log in position order.
    pub events: Vec<StoredEvent>,
}
