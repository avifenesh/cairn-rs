use cairn_domain::ProjectKey;

use crate::error::FabricError;

/// Current wall-clock time in milliseconds since UNIX_EPOCH. On clock skew
/// (system clock set before 1970), logs a warning and returns 0 so the
/// caller at least sees an obviously-wrong timestamp rather than silently
/// continuing with `Duration::default()`.
pub fn now_ms() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as u64,
        Err(e) => {
            tracing::warn!(error = %e, "system clock is before UNIX_EPOCH — returning 0");
            0
        }
    }
}

pub fn check_fcall_success(raw: &ferriskey::Value, function_name: &str) -> Result<(), FabricError> {
    let arr = match raw {
        ferriskey::Value::Array(arr) => arr,
        _ => return Ok(()),
    };
    let status = match arr.first() {
        Some(Ok(ferriskey::Value::Int(n))) => *n,
        _ => return Ok(()),
    };
    if status == 1 {
        return Ok(());
    }
    let code = match arr.get(1) {
        Some(Ok(ferriskey::Value::BulkString(b))) => String::from_utf8_lossy(b).into_owned(),
        Some(Ok(ferriskey::Value::SimpleString(s))) => s.clone(),
        _ => "unknown".to_owned(),
    };
    Err(FabricError::Internal(format!(
        "{function_name} rejected: {code}"
    )))
}

/// Extract the Lua error code string from a rejected fcall envelope
/// (`{Int(status_code), BulkString(error_code), ...}`). Returns `None` when
/// the envelope is OK or malformed.
///
/// Callers use this to dispatch on FF's typed error codes (e.g.
/// `use_claim_resumed_execution`) without going through the string-formatted
/// [`FabricError::Internal`] message. Keep the caller pattern:
///
/// ```ignore
/// if let Some(code) = fcall_error_code(&raw) {
///     if code == "use_claim_resumed_execution" { /* dispatch */ }
/// }
/// check_fcall_success(&raw, FF_…)?;
/// ```
pub fn fcall_error_code(raw: &ferriskey::Value) -> Option<String> {
    let arr = match raw {
        ferriskey::Value::Array(arr) => arr,
        _ => return None,
    };
    let status = match arr.first() {
        Some(Ok(ferriskey::Value::Int(n))) => *n,
        _ => return None,
    };
    if status == 1 {
        return None;
    }
    match arr.get(1) {
        Some(Ok(ferriskey::Value::BulkString(b))) => Some(String::from_utf8_lossy(b).into_owned()),
        Some(Ok(ferriskey::Value::SimpleString(s))) => Some(s.clone()),
        _ => None,
    }
}

pub fn parse_public_state(s: &str) -> flowfabric::core::state::PublicState {
    match s {
        "waiting" => flowfabric::core::state::PublicState::Waiting,
        "delayed" => flowfabric::core::state::PublicState::Delayed,
        "rate_limited" => flowfabric::core::state::PublicState::RateLimited,
        "waiting_children" => flowfabric::core::state::PublicState::WaitingChildren,
        "active" => flowfabric::core::state::PublicState::Active,
        "suspended" => flowfabric::core::state::PublicState::Suspended,
        "completed" => flowfabric::core::state::PublicState::Completed,
        "failed" => flowfabric::core::state::PublicState::Failed,
        "cancelled" => flowfabric::core::state::PublicState::Cancelled,
        "expired" => flowfabric::core::state::PublicState::Expired,
        "skipped" => flowfabric::core::state::PublicState::Skipped,
        _ => flowfabric::core::state::PublicState::Waiting,
    }
}

pub fn try_parse_project_key(s: &str) -> Option<ProjectKey> {
    let parts: Vec<&str> = s.splitn(3, '/').collect();
    match parts.as_slice() {
        [t, w, p] if !t.is_empty() && !w.is_empty() && !p.is_empty() => {
            Some(ProjectKey::new(*t, *w, *p))
        }
        _ => None,
    }
}

pub fn read_hgetall_field(
    fields: &std::collections::HashMap<String, String>,
    key: &str,
) -> Option<String> {
    fields.get(key).filter(|v| !v.is_empty()).cloned()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailOutcome {
    RetryScheduled,
    TerminalFailed,
}

pub fn is_already_satisfied(raw: &ferriskey::Value) -> bool {
    if let ferriskey::Value::Array(arr) = raw {
        if let Some(Ok(ferriskey::Value::BulkString(b))) = arr.get(1) {
            return &**b == b"ALREADY_SATISFIED";
        }
        if let Some(Ok(ferriskey::Value::SimpleString(s))) = arr.get(1) {
            return s == "ALREADY_SATISFIED";
        }
    }
    false
}

pub fn parse_fail_outcome(raw: &ferriskey::Value) -> FailOutcome {
    if let ferriskey::Value::Array(arr) = raw {
        if let Some(Ok(ferriskey::Value::BulkString(b))) = arr.get(2) {
            if &**b == b"retry_scheduled" {
                return FailOutcome::RetryScheduled;
            }
        }
        if let Some(Ok(ferriskey::Value::SimpleString(s))) = arr.get(2) {
            if s == "retry_scheduled" {
                return FailOutcome::RetryScheduled;
            }
        }
    }
    FailOutcome::TerminalFailed
}

pub fn sanitize_signal_component(s: &str) -> String {
    s.replace(':', "_")
}

/// Extract a `String` out of a ferriskey `Value` in bulk or simple
/// string form. Returns `None` for other shapes.
pub fn value_to_string(v: &ferriskey::Value) -> Option<String> {
    match v {
        ferriskey::Value::BulkString(b) => Some(String::from_utf8_lossy(b).into_owned()),
        ferriskey::Value::SimpleString(s) => Some(s.clone()),
        ferriskey::Value::VerbatimString { text, .. } => Some(text.clone()),
        _ => None,
    }
}

/// Flatten a collection of string-like `Value`s into a `Vec<String>`.
/// Handles both `Value::Array(Vec<Result<Value, _>>)` (LRANGE, HKEYS…)
/// and `Value::Set(Vec<Value>)` (SMEMBERS, SINTER…). Errored or
/// non-string entries are skipped silently — missing/garbled members
/// shouldn't crash the read.
pub fn parse_string_array(raw: &ferriskey::Value) -> Vec<String> {
    match raw {
        ferriskey::Value::Array(items) => items
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .filter_map(value_to_string)
            .collect(),
        ferriskey::Value::Set(items) => items.iter().filter_map(value_to_string).collect(),
        _ => Vec::new(),
    }
}

/// Extract the `new_graph_revision` from the
/// `ff_stage_dependency_edge` OK envelope
/// `[1, "OK", "<edge_id>", "<new_graph_revision>"]`. FF's `ok(...)`
/// helper (flowfabric.lua) wraps `(status=1, "OK", ...caller_args)`,
/// so index 3 carries the second caller-supplied value. Returns
/// `None` on malformed shape.
pub fn parse_stage_result_revision(raw: &ferriskey::Value) -> Option<u64> {
    let ferriskey::Value::Array(arr) = raw else {
        return None;
    };
    let rev_value = arr.get(3)?.as_ref().ok()?;
    value_to_string(rev_value).and_then(|s| s.parse().ok())
}

/// Extract the eligibility state string from the
/// `ff_evaluate_flow_eligibility` OK envelope `[1, "OK", "<state>"]`.
/// Returns `None` on malformed shape.
pub fn parse_eligibility_result(raw: &ferriskey::Value) -> Option<String> {
    let ferriskey::Value::Array(arr) = raw else {
        return None;
    };
    let state_value = arr.get(2)?.as_ref().ok()?;
    value_to_string(state_value)
}

pub fn is_duplicate_result(raw: &ferriskey::Value) -> bool {
    if let ferriskey::Value::Array(arr) = raw {
        if let Some(Ok(ferriskey::Value::BulkString(b))) = arr.get(1) {
            return &**b == b"DUPLICATE";
        }
        if let Some(Ok(ferriskey::Value::SimpleString(s))) = arr.get(1) {
            return s == "DUPLICATE";
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_public_state_all_variants() {
        assert_eq!(
            parse_public_state("waiting"),
            flowfabric::core::state::PublicState::Waiting
        );
        assert_eq!(
            parse_public_state("active"),
            flowfabric::core::state::PublicState::Active
        );
        assert_eq!(
            parse_public_state("completed"),
            flowfabric::core::state::PublicState::Completed
        );
        assert_eq!(
            parse_public_state("failed"),
            flowfabric::core::state::PublicState::Failed
        );
        assert_eq!(
            parse_public_state("cancelled"),
            flowfabric::core::state::PublicState::Cancelled
        );
        assert_eq!(
            parse_public_state("suspended"),
            flowfabric::core::state::PublicState::Suspended
        );
        assert_eq!(
            parse_public_state("expired"),
            flowfabric::core::state::PublicState::Expired
        );
        assert_eq!(
            parse_public_state("skipped"),
            flowfabric::core::state::PublicState::Skipped
        );
        assert_eq!(
            parse_public_state("delayed"),
            flowfabric::core::state::PublicState::Delayed
        );
        assert_eq!(
            parse_public_state("rate_limited"),
            flowfabric::core::state::PublicState::RateLimited
        );
        assert_eq!(
            parse_public_state("waiting_children"),
            flowfabric::core::state::PublicState::WaitingChildren
        );
        assert_eq!(
            parse_public_state("garbage"),
            flowfabric::core::state::PublicState::Waiting
        );
    }

    #[test]
    fn try_parse_project_key_valid() {
        let pk = try_parse_project_key("t/w/p").unwrap();
        assert_eq!(pk.tenant_id.as_str(), "t");
        assert_eq!(pk.workspace_id.as_str(), "w");
        assert_eq!(pk.project_id.as_str(), "p");
    }

    #[test]
    fn try_parse_project_key_with_slashes() {
        let pk = try_parse_project_key("t/w/p/extra").unwrap();
        assert_eq!(pk.project_id.as_str(), "p/extra");
    }

    #[test]
    fn try_parse_project_key_invalid_returns_none() {
        assert!(try_parse_project_key("bad").is_none());
    }

    #[test]
    fn try_parse_project_key_empty_returns_none() {
        assert!(try_parse_project_key("").is_none());
    }

    #[test]
    fn try_parse_project_key_empty_parts_returns_none() {
        assert!(try_parse_project_key("t//p").is_none());
        assert!(try_parse_project_key("/w/p").is_none());
    }

    #[test]
    fn is_duplicate_detects_duplicate_simple_string() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("DUPLICATE".to_owned())),
        ]);
        assert!(is_duplicate_result(&raw));
    }

    #[test]
    fn is_duplicate_detects_duplicate_bulk_string() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::BulkString(b"DUPLICATE".to_vec().into())),
        ]);
        assert!(is_duplicate_result(&raw));
    }

    #[test]
    fn is_duplicate_returns_false_for_ok() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
        ]);
        assert!(!is_duplicate_result(&raw));
    }

    #[test]
    fn is_duplicate_returns_false_for_non_array() {
        let raw = ferriskey::Value::SimpleString("not an array".to_owned());
        assert!(!is_duplicate_result(&raw));
    }

    #[test]
    fn is_duplicate_returns_false_for_empty_array() {
        let raw = ferriskey::Value::Array(vec![]);
        assert!(!is_duplicate_result(&raw));
    }

    #[test]
    fn check_fcall_success_ok() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
        ]);
        assert!(check_fcall_success(&raw, "test").is_ok());
    }

    #[test]
    fn check_fcall_success_error_returns_err() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(0)),
            Ok(ferriskey::Value::SimpleString("lease_expired".to_owned())),
        ]);
        let err = check_fcall_success(&raw, "ff_complete_execution");
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("lease_expired"));
    }

    #[test]
    fn check_fcall_success_error_bulk_string() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(0)),
            Ok(ferriskey::Value::BulkString(b"stale_lease".to_vec().into())),
        ]);
        let err = check_fcall_success(&raw, "ff_cancel");
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("stale_lease"));
    }

    #[test]
    fn check_fcall_success_non_array_passes() {
        let raw = ferriskey::Value::SimpleString("OK".to_owned());
        assert!(check_fcall_success(&raw, "test").is_ok());
    }

    #[test]
    fn check_fcall_success_duplicate_is_ok() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("DUPLICATE".to_owned())),
        ]);
        assert!(check_fcall_success(&raw, "test").is_ok());
    }

    #[test]
    fn parse_fail_outcome_retry_scheduled_simple_string() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
            Ok(ferriskey::Value::SimpleString("retry_scheduled".to_owned())),
            Ok(ferriskey::Value::SimpleString("1234567890".to_owned())),
        ]);
        assert_eq!(parse_fail_outcome(&raw), FailOutcome::RetryScheduled);
    }

    #[test]
    fn parse_fail_outcome_retry_scheduled_bulk_string() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
            Ok(ferriskey::Value::BulkString(
                b"retry_scheduled".to_vec().into(),
            )),
        ]);
        assert_eq!(parse_fail_outcome(&raw), FailOutcome::RetryScheduled);
    }

    #[test]
    fn parse_fail_outcome_terminal_failed() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
            Ok(ferriskey::Value::SimpleString("terminal_failed".to_owned())),
        ]);
        assert_eq!(parse_fail_outcome(&raw), FailOutcome::TerminalFailed);
    }

    #[test]
    fn parse_fail_outcome_non_array_defaults_terminal() {
        let raw = ferriskey::Value::SimpleString("OK".to_owned());
        assert_eq!(parse_fail_outcome(&raw), FailOutcome::TerminalFailed);
    }

    #[test]
    fn parse_fail_outcome_short_array_defaults_terminal() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
        ]);
        assert_eq!(parse_fail_outcome(&raw), FailOutcome::TerminalFailed);
    }

    #[test]
    fn check_fcall_success_already_satisfied_is_ok() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString(
                "ALREADY_SATISFIED".to_owned(),
            )),
        ]);
        assert!(check_fcall_success(&raw, "test").is_ok());
    }

    #[test]
    fn is_already_satisfied_simple_string() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString(
                "ALREADY_SATISFIED".to_owned(),
            )),
        ]);
        assert!(is_already_satisfied(&raw));
    }

    #[test]
    fn is_already_satisfied_bulk_string() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::BulkString(
                b"ALREADY_SATISFIED".to_vec().into(),
            )),
        ]);
        assert!(is_already_satisfied(&raw));
    }

    #[test]
    fn is_already_satisfied_false_for_ok() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
        ]);
        assert!(!is_already_satisfied(&raw));
    }

    #[test]
    fn is_already_satisfied_false_for_non_array() {
        let raw = ferriskey::Value::SimpleString("OK".to_owned());
        assert!(!is_already_satisfied(&raw));
    }
}
