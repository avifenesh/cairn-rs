pub const EXECUTION_KIND_RUN: &str = "cairn_run";
pub const EXECUTION_KIND_TASK: &str = "cairn_task";
pub const SOURCE_IDENTITY: &str = "cairn";
pub const CANCEL_REASON_OPERATOR: &str = "operator_cancel";
pub const CANCEL_SOURCE_OVERRIDE: &str = "operator_override";

// Only waiting_for_approval is actively checked (by state_map to distinguish
// WaitingApproval from Paused). Other FF blocking_reason values pass through
// exec_core but cairn doesn't branch on them.
pub const BLOCKING_WAITING_FOR_APPROVAL: &str = "waiting_for_approval";

pub const DEFAULT_LEASE_HISTORY_MAXLEN: &str = "1000";
pub const DEFAULT_LEASE_HISTORY_GRACE_MS: &str = "5000";
pub const DEFAULT_SIGNAL_MAXLEN: &str = "1000";
pub const DEFAULT_MAX_SIGNALS_PER_EXECUTION: &str = "10000";
