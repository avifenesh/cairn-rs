//! Default deny patterns fed into `PermissionPolicy::with_sensitive_patterns`.
//!
//! Mirrors the TS `@agent-sh/harness-*` defaults. Tools check these globs
//! before opening / reading / writing files and fail with
//! `ToolErrorCode::Sensitive` on match.

/// Globs the harness layer rejects by default. Override via session config
/// when a specific cairn profile needs narrower or wider fencing.
pub fn default_sensitive_patterns() -> Vec<String> {
    vec![
        "**/.env".into(),
        "**/.env.*".into(),
        "**/*.pem".into(),
        "**/*.key".into(),
        "**/secrets/**".into(),
        "**/.ssh/**".into(),
        "**/credentials.json".into(),
        "**/.aws/credentials".into(),
        "**/id_rsa".into(),
        "**/id_ed25519".into(),
    ]
}
