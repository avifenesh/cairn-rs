//! shell_exec — Run a shell command in a subprocess.
//!
//! **ExecutionClass::Sensitive** — the orchestrator MUST gate every invocation
//! through `ApprovalService` before the command is dispatched.
//!
//! ## Parameters
//! ```json
//! { "command": "ls -la /tmp", "timeout_ms": 30000, "working_dir": "/home/agent" }
//! ```
//!
//! ## Output
//! ```json
//! { "exit_code": 0, "stdout": "...", "stderr": "", "timed_out": false }
//! ```
//!
//! stdout and stderr are each capped at **16 KB**.  Default timeout is **30 s**.

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey};
use serde_json::Value;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Shell command execution tool.
///
/// Runs via `/bin/sh -c <command>` so shell operators (`&&`, `|`, `;`) work.
/// Tagged `Sensitive` — approval required before dispatch.
pub struct ShellExecTool;

impl Default for ShellExecTool {
    fn default() -> Self { Self }
}

#[async_trait]
impl ToolHandler for ShellExecTool {
    fn name(&self) -> &str { "shell_exec" }

    fn tier(&self) -> ToolTier { ToolTier::Registered }

    fn description(&self) -> &str {
        "Run a shell command in a subprocess. SENSITIVE — requires operator approval."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command":     { "type": "string",
                                 "description": "Shell command (run via /bin/sh -c)." },
                "timeout_ms":  { "type": "integer", "default": 30000,
                                 "description": "Max execution time in milliseconds." },
                "working_dir": { "type": "string",
                                 "description": "Working directory (default: process cwd)." }
            }
        })
    }

    /// SENSITIVE — orchestrator gates every call through ApprovalService.
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::Sensitive
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        // ── Validate ──────────────────────────────────────────────────────────
        let command = args.get("command")
            .and_then(|c| c.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "command".into(),
                message: "required".into(),
            })?;

        if command.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "command".into(),
                message: "must not be empty".into(),
            });
        }

        let timeout_ms = args.get("timeout_ms").and_then(|t| t.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS);

        // ── Spawn ─────────────────────────────────────────────────────────────
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(command);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if let Some(dir) = args.get("working_dir").and_then(|d| d.as_str()) {
            cmd.current_dir(dir);
        }

        // ── Run with timeout ──────────────────────────────────────────────────
        match timeout(Duration::from_millis(timeout_ms), cmd.output()).await {
            Err(_elapsed) => Ok(ToolResult::ok(serde_json::json!({
                "exit_code": null,
                "stdout":    "",
                "stderr":    "",
                "timed_out": true,
            }))),
            Ok(Err(e)) => Err(ToolError::Transient(format!("spawn failed: {e}"))),
            Ok(Ok(output)) => {
                let exit_code = output.status.code();
                let stdout_raw = &output.stdout;
                let stderr_raw = &output.stderr;
                let stdout_truncated = stdout_raw.len() > MAX_OUTPUT_BYTES;
                let stderr_truncated = stderr_raw.len() > MAX_OUTPUT_BYTES;

                let out = serde_json::json!({
                    "exit_code": exit_code,
                    "stdout":    cap(stdout_raw),
                    "stderr":    cap(stderr_raw),
                    "timed_out": false,
                });

                Ok(if stdout_truncated || stderr_truncated {
                    ToolResult::truncated(out)
                } else {
                    ToolResult::ok(out)
                })
            }
        }
    }
}

fn cap(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_OUTPUT_BYTES)]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> ProjectKey { ProjectKey::new("t", "w", "p") }

    // ── Metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn name_tier() {
        assert_eq!(ShellExecTool.name(), "shell_exec");
        assert_eq!(ShellExecTool.tier(), ToolTier::Registered);
    }

    #[test]
    fn execution_class_is_sensitive() {
        assert_eq!(
            ShellExecTool.execution_class(),
            ExecutionClass::Sensitive,
            "shell_exec must be Sensitive so the orchestrator gates via approval"
        );
    }

    #[test]
    fn schema_requires_command() {
        let req = ShellExecTool.parameters_schema()["required"]
            .as_array().unwrap().clone();
        assert!(req.iter().any(|v| v.as_str() == Some("command")));
    }

    // ── Validation errors ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn missing_command_is_invalid_args() {
        let err = ShellExecTool
            .execute(&project(), serde_json::json!({}))
            .await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn empty_command_is_invalid_args() {
        let err = ShellExecTool
            .execute(&project(), serde_json::json!({"command": "  "}))
            .await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    // ── Happy paths ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn echo_captures_stdout() {
        let result = ShellExecTool
            .execute(&project(), serde_json::json!({"command": "echo hello"}))
            .await.unwrap();
        assert_eq!(result.output["exit_code"], 0);
        assert!(result.output["stdout"].as_str().unwrap_or("").contains("hello"));
        assert!(!result.output["timed_out"].as_bool().unwrap_or(true));
    }

    #[tokio::test]
    async fn nonzero_exit_code_captured() {
        let result = ShellExecTool
            .execute(&project(), serde_json::json!({"command": "exit 42"}))
            .await.unwrap();
        assert_eq!(result.output["exit_code"], 42);
    }

    #[tokio::test]
    async fn stderr_captured() {
        let result = ShellExecTool
            .execute(&project(), serde_json::json!({"command": "echo err >&2"}))
            .await.unwrap();
        assert!(result.output["stderr"].as_str().unwrap_or("").contains("err"));
    }

    #[tokio::test]
    async fn timeout_returns_timed_out_true() {
        let result = ShellExecTool
            .execute(&project(), serde_json::json!({"command": "sleep 60", "timeout_ms": 50}))
            .await.unwrap();
        assert!(result.output["timed_out"].as_bool().unwrap_or(false));
    }

    #[tokio::test]
    async fn pipeline_operators_work() {
        let result = ShellExecTool
            .execute(&project(), serde_json::json!({"command": "echo hello | tr a-z A-Z"}))
            .await.unwrap();
        assert!(result.output["stdout"].as_str().unwrap_or("").contains("HELLO"));
    }

    // ── Output capping ────────────────────────────────────────────────────────

    #[test]
    fn cap_truncates_at_limit() {
        let big = b"x".repeat(MAX_OUTPUT_BYTES + 100);
        assert_eq!(cap(&big).len(), MAX_OUTPUT_BYTES);
    }

    #[test]
    fn cap_short_input_is_unchanged() {
        assert_eq!(cap(b"hi"), "hi");
    }
}
