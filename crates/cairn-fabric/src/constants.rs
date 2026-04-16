pub const EXECUTION_KIND_RUN: &str = "cairn_run";
pub const EXECUTION_KIND_TASK: &str = "cairn_task";
pub const SOURCE_IDENTITY: &str = "cairn";
pub const CANCEL_REASON_OPERATOR: &str = "operator_cancel";
pub const CANCEL_SOURCE_OVERRIDE: &str = "operator_override";
pub const RESUME_TRIGGER_OPERATOR: &str = "operator";

// FF blocking_reason strings (coupled to FF Lua helpers.lua REASON_TO_BLOCKING table)
pub const BLOCKING_WAITING_FOR_APPROVAL: &str = "waiting_for_approval";
pub const BLOCKING_WAITING_FOR_SIGNAL: &str = "waiting_for_signal";
pub const BLOCKING_WAITING_FOR_TOOL_RESULT: &str = "waiting_for_tool_result";
pub const BLOCKING_WAITING_FOR_CHILDREN: &str = "waiting_for_children";
pub const BLOCKING_PAUSED_BY_OPERATOR: &str = "paused_by_operator";
pub const BLOCKING_PAUSED_BY_POLICY: &str = "paused_by_policy";

pub const DEFAULT_GRANT_TTL_MS: u64 = 5000;
pub const DEFAULT_LEASE_HISTORY_MAXLEN: &str = "1000";
pub const DEFAULT_SIGNAL_MAXLEN: &str = "1000";
pub const DEFAULT_MAX_SIGNALS_PER_EXECUTION: &str = "10000";
