//! Recovery escalation domain types per RFC 005.

use crate::RunId;
use serde::{Deserialize, Serialize};

/// Persistent record of a run that exceeded the recovery attempt threshold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryEscalation {
    pub run_id: RunId,
    pub attempt_count: u32,
    pub last_error: String,
    pub escalated_at_ms: u64,
}
