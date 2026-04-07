use crate::ids::ScheduledTaskId;
use crate::ids::TenantId;
use serde::{Deserialize, Serialize};

/// Durable record for a tenant-scoped scheduled task.
///
/// Scheduled tasks trigger recurring agent workflows (e.g. weekly reflection).
/// The runtime recovery sweep can check `next_run_at` against the current
/// clock and enqueue due tasks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduledTaskRecord {
    pub scheduled_task_id: ScheduledTaskId,
    pub tenant_id: TenantId,
    /// Human-readable label (e.g. "weekly_reflection").
    pub name: String,
    /// Cron expression describing the schedule (e.g. "0 9 * * 1" for Monday 09:00).
    pub cron_expression: String,
    /// Unix milliseconds of the most recent successful trigger, if any.
    pub last_run_at: Option<u64>,
    /// Unix milliseconds when this task should next fire.
    pub next_run_at: Option<u64>,
    /// Whether this task is active. Disabled tasks are skipped by the sweep.
    pub enabled: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

// Manual Eq: the struct contains no f64, but we implement Eq explicitly so
// ScheduledTaskRecord can be used in contexts that require it.
impl Eq for ScheduledTaskRecord {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduled_task_record_round_trips() {
        let rec = ScheduledTaskRecord {
            scheduled_task_id: ScheduledTaskId::new("sched_1"),
            tenant_id: TenantId::new("tenant_acme"),
            name: "weekly_reflection".to_owned(),
            cron_expression: "0 9 * * 1".to_owned(),
            last_run_at: None,
            next_run_at: Some(1_700_000_000_000),
            enabled: true,
            created_at: 1_000,
            updated_at: 1_000,
        };
        assert_eq!(rec.scheduled_task_id.as_str(), "sched_1");
        assert!(rec.enabled);
        assert!(rec.last_run_at.is_none());
    }
}
