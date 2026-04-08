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
