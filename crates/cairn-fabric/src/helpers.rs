use cairn_domain::ProjectKey;

pub fn parse_public_state(s: &str) -> ff_core::state::PublicState {
    match s {
        "waiting" => ff_core::state::PublicState::Waiting,
        "delayed" => ff_core::state::PublicState::Delayed,
        "rate_limited" => ff_core::state::PublicState::RateLimited,
        "waiting_children" => ff_core::state::PublicState::WaitingChildren,
        "active" => ff_core::state::PublicState::Active,
        "suspended" => ff_core::state::PublicState::Suspended,
        "completed" => ff_core::state::PublicState::Completed,
        "failed" => ff_core::state::PublicState::Failed,
        "cancelled" => ff_core::state::PublicState::Cancelled,
        "expired" => ff_core::state::PublicState::Expired,
        "skipped" => ff_core::state::PublicState::Skipped,
        _ => ff_core::state::PublicState::Waiting,
    }
}

pub fn parse_project_key(s: &str) -> ProjectKey {
    let parts: Vec<&str> = s.splitn(3, '/').collect();
    match parts.as_slice() {
        [t, w, p] => ProjectKey::new(*t, *w, *p),
        _ => ProjectKey::new("default_tenant", "default_workspace", "default_project"),
    }
}

pub fn read_hgetall_field(
    fields: &std::collections::HashMap<String, String>,
    key: &str,
) -> Option<String> {
    fields.get(key).filter(|v| !v.is_empty()).cloned()
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
            ff_core::state::PublicState::Waiting
        );
        assert_eq!(
            parse_public_state("active"),
            ff_core::state::PublicState::Active
        );
        assert_eq!(
            parse_public_state("completed"),
            ff_core::state::PublicState::Completed
        );
        assert_eq!(
            parse_public_state("failed"),
            ff_core::state::PublicState::Failed
        );
        assert_eq!(
            parse_public_state("cancelled"),
            ff_core::state::PublicState::Cancelled
        );
        assert_eq!(
            parse_public_state("suspended"),
            ff_core::state::PublicState::Suspended
        );
        assert_eq!(
            parse_public_state("expired"),
            ff_core::state::PublicState::Expired
        );
        assert_eq!(
            parse_public_state("skipped"),
            ff_core::state::PublicState::Skipped
        );
        assert_eq!(
            parse_public_state("delayed"),
            ff_core::state::PublicState::Delayed
        );
        assert_eq!(
            parse_public_state("rate_limited"),
            ff_core::state::PublicState::RateLimited
        );
        assert_eq!(
            parse_public_state("waiting_children"),
            ff_core::state::PublicState::WaitingChildren
        );
        assert_eq!(
            parse_public_state("garbage"),
            ff_core::state::PublicState::Waiting
        );
    }

    #[test]
    fn parse_project_key_valid() {
        let pk = parse_project_key("t/w/p");
        assert_eq!(pk.tenant_id.as_str(), "t");
        assert_eq!(pk.workspace_id.as_str(), "w");
        assert_eq!(pk.project_id.as_str(), "p");
    }

    #[test]
    fn parse_project_key_with_slashes() {
        let pk = parse_project_key("t/w/p/extra");
        assert_eq!(pk.project_id.as_str(), "p/extra");
    }

    #[test]
    fn parse_project_key_invalid() {
        let pk = parse_project_key("bad");
        assert_eq!(pk.tenant_id.as_str(), "default_tenant");
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
}
