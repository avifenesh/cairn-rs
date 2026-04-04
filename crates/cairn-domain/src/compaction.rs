//! Event log compaction and snapshot types per RFC 002.

use crate::TenantId;
use serde::{Deserialize, Serialize};

/// Summary returned after compacting the event log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionReport {
    /// Total events before compaction.
    pub events_before: u64,
    /// Total events retained after compaction.
    pub events_after: u64,
    /// Number of entities whose projections were recomputed.
    pub entities_recomputed: u32,
}

/// A point-in-time snapshot of the event log for a tenant.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub snapshot_id: String,
    pub tenant_id: TenantId,
    /// Position of the last event captured in this snapshot.
    pub event_position: u64,
    /// FNV-64 hash of the compressed_state bytes (hex string).
    pub state_hash: String,
    pub created_at_ms: u64,
    /// JSON-serialized tenant event log at snapshot time.
    pub compressed_state: Vec<u8>,
}
