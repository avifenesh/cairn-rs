use serde::{Deserialize, Serialize};

use crate::TenantId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantQuota {
    pub tenant_id: TenantId,
    pub max_concurrent_runs: u32,
    pub max_sessions_per_hour: u32,
    pub max_tasks_per_run: u32,
    pub current_active_runs: u32,
    pub sessions_this_hour: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub policy_id: String,
    pub tenant_id: TenantId,
    pub full_history_days: u32,
    pub current_state_days: u32,
    pub max_events_per_entity: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionResult {
    pub events_pruned: u64,
    pub entities_affected: u32,
}
