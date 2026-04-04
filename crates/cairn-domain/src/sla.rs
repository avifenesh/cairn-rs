//! Run SLA (Service Level Agreement) tracking types per RFC 005.

use crate::{RunId, TenantId};
use serde::{Deserialize, Serialize};

/// SLA configuration set for a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlaConfig {
    pub run_id: RunId,
    pub tenant_id: TenantId,
    pub target_completion_ms: u64,
    pub alert_at_percent: u8,
    pub configured_at_ms: u64,
}

/// Current SLA status for a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlaStatus {
    pub on_track: bool,
    pub elapsed_ms: u64,
    pub target_ms: u64,
    /// Percentage of the SLA target consumed (may exceed 100 when breached).
    pub percent_used: u64,
}

/// Persisted record of an SLA breach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlaBreach {
    pub run_id: RunId,
    pub tenant_id: TenantId,
    pub elapsed_ms: u64,
    pub target_ms: u64,
    pub breached_at_ms: u64,
}
