//! Request validation helpers for POST endpoints.
//!
//! Simple constraint checks — no external crate dependencies.
//! Each function returns `Ok(())` or `Err(String)` with a clear message.

/// Maximum length for identifier fields (tenant_id, session_id, run_id, etc.).
pub const MAX_ID_LEN: usize = 128;
/// Maximum length for name fields.
#[allow(dead_code)]
pub const MAX_NAME_LEN: usize = 256;
/// Maximum length for description / reason fields.
pub const MAX_DESC_LEN: usize = 4096;
/// Maximum length for free-text prompt fields.
pub const MAX_PROMPT_LEN: usize = 100_000;

/// Validate that a required string field is present and non-empty.
#[allow(dead_code)]
pub fn require(field: &str, value: &Option<String>) -> Result<(), String> {
    match value {
        Some(v) if !v.trim().is_empty() => Ok(()),
        _ => Err(format!("{field} is required")),
    }
}

/// Validate that a string (if present) does not exceed `max` bytes.
pub fn max_len(field: &str, value: &Option<String>, max: usize) -> Result<(), String> {
    if let Some(v) = value {
        if v.len() > max {
            return Err(format!(
                "{field} exceeds maximum length of {max} characters"
            ));
        }
    }
    Ok(())
}

/// Validate that a non-optional string does not exceed `max` bytes.
pub fn max_len_str(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max {
        Err(format!(
            "{field} exceeds maximum length of {max} characters"
        ))
    } else {
        Ok(())
    }
}

/// Validate an ID field: present, non-empty, within length, no control chars.
pub fn valid_id(field: &str, value: &Option<String>) -> Result<(), String> {
    let v = value.as_deref().unwrap_or("");
    if v.is_empty() {
        return Ok(()); // optional IDs are allowed to be absent
    }
    if v.len() > MAX_ID_LEN {
        return Err(format!(
            "{field} exceeds maximum length of {MAX_ID_LEN} characters"
        ));
    }
    if v.chars().any(|c| c.is_control()) {
        return Err(format!("{field} must not contain control characters"));
    }
    Ok(())
}

/// Validate a required ID field.
pub fn require_id(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() > MAX_ID_LEN {
        return Err(format!(
            "{field} exceeds maximum length of {MAX_ID_LEN} characters"
        ));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(format!("{field} must not contain control characters"));
    }
    Ok(())
}

/// Validate that a numeric value is positive (> 0).
#[allow(dead_code)]
pub fn positive_u64(field: &str, value: u64) -> Result<(), String> {
    if value == 0 {
        Err(format!("{field} must be greater than 0"))
    } else {
        Ok(())
    }
}

/// Collect all validation errors and return the first failure (short-circuit).
pub fn check_all(checks: &[Result<(), String>]) -> Result<(), String> {
    for check in checks {
        if let Err(msg) = check {
            return Err(msg.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── valid_id ───────────────────────────────────────────────────────

    #[test]
    fn valid_id_accepts_normal_id() {
        assert!(valid_id("run_id", &Some("run-abc-123".into())).is_ok());
    }

    #[test]
    fn valid_id_allows_absent() {
        // None is treated as optional → ok
        assert!(valid_id("run_id", &None).is_ok());
    }

    #[test]
    fn valid_id_allows_empty_string() {
        // Empty string treated same as absent
        assert!(valid_id("run_id", &Some(String::new())).is_ok());
    }

    #[test]
    fn valid_id_rejects_too_long() {
        let long = "x".repeat(MAX_ID_LEN + 1);
        let result = valid_id("run_id", &Some(long));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("maximum length"));
    }

    #[test]
    fn valid_id_accepts_at_max_len() {
        let exact = "a".repeat(MAX_ID_LEN);
        assert!(valid_id("run_id", &Some(exact)).is_ok());
    }

    #[test]
    fn valid_id_rejects_control_characters() {
        let with_null = "run-\x00-id".to_string();
        let result = valid_id("run_id", &Some(with_null));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("control characters"));
    }

    #[test]
    fn valid_id_rejects_newline() {
        let with_newline = "run\nid".to_string();
        let result = valid_id("run_id", &Some(with_newline));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("control characters"));
    }

    #[test]
    fn valid_id_rejects_tab() {
        let result = valid_id("f", &Some("a\tb".into()));
        assert!(result.is_err());
    }

    // ── require_id ─────────────────────────────────────────────────────

    #[test]
    fn require_id_accepts_normal() {
        assert!(require_id("tenant_id", "default_tenant").is_ok());
    }

    #[test]
    fn require_id_rejects_empty() {
        let result = require_id("tenant_id", "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("required"));
    }

    #[test]
    fn require_id_rejects_whitespace_only() {
        let result = require_id("tenant_id", "   ");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("required"));
    }

    #[test]
    fn require_id_rejects_too_long() {
        let long = "z".repeat(MAX_ID_LEN + 1);
        let result = require_id("tenant_id", &long);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("maximum length"));
    }

    #[test]
    fn require_id_rejects_control_chars() {
        let result = require_id("tenant_id", "ok\x07id");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("control characters"));
    }

    // ── require ────────────────────────────────────────────────────────

    #[test]
    fn require_accepts_present_value() {
        assert!(require("name", &Some("hello".into())).is_ok());
    }

    #[test]
    fn require_rejects_none() {
        let result = require("name", &None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("required"));
    }

    #[test]
    fn require_rejects_whitespace_only() {
        let result = require("name", &Some("   ".into()));
        assert!(result.is_err());
    }

    // ── max_len (Option<String>) ───────────────────────────────────────

    #[test]
    fn max_len_ok_within_limit() {
        assert!(max_len("desc", &Some("short".into()), 100).is_ok());
    }

    #[test]
    fn max_len_ok_when_none() {
        assert!(max_len("desc", &None, 100).is_ok());
    }

    #[test]
    fn max_len_rejects_over_limit() {
        let result = max_len("desc", &Some("x".repeat(101)), 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("maximum length"));
    }

    #[test]
    fn max_len_ok_at_exact_limit() {
        assert!(max_len("desc", &Some("x".repeat(100)), 100).is_ok());
    }

    // ── max_len_str ────────────────────────────────────────────────────

    #[test]
    fn max_len_str_ok_within() {
        assert!(max_len_str("field", "hello", 10).is_ok());
    }

    #[test]
    fn max_len_str_rejects_over() {
        assert!(max_len_str("field", &"x".repeat(11), 10).is_err());
    }

    // ── positive_u64 ───────────────────────────────────────────────────

    #[test]
    fn positive_u64_accepts_nonzero() {
        assert!(positive_u64("count", 1).is_ok());
        assert!(positive_u64("count", 999).is_ok());
    }

    #[test]
    fn positive_u64_rejects_zero() {
        let result = positive_u64("count", 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("greater than 0"));
    }

    // ── check_all ──────────────────────────────────────────────────────

    #[test]
    fn check_all_passes_when_all_ok() {
        assert!(check_all(&[Ok(()), Ok(()), Ok(())]).is_ok());
    }

    #[test]
    fn check_all_returns_first_error() {
        let checks = vec![
            Ok(()),
            Err("first error".to_string()),
            Err("second error".to_string()),
        ];
        let result = check_all(&checks);
        assert_eq!(result.unwrap_err(), "first error");
    }

    #[test]
    fn check_all_passes_on_empty() {
        assert!(check_all(&[]).is_ok());
    }
}
