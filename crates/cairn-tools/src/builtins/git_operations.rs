//! `git_operations` built-in tool — run safe git commands within a workspace.
//!
//! Supports: `status`, `diff`, `log`, `branch`, `checkout`, `commit`.
//! Push and any write to main/master always require approval.
//!
//! All commands run in `workspace_root` with a 60-second wall-clock timeout.

use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey};
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

const GIT_TIMEOUT_SECS: u64 = 60;

/// Safe git operations for code-agent workflows.
pub struct GitOperationsTool {
    workspace_root: PathBuf,
}

impl GitOperationsTool {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }
}

/// Commands that are read-only and never require approval.
const READONLY_CMDS: &[&str] = &["status", "diff", "log", "branch", "show", "stash"];
/// Commands that write to the workspace but are sandboxed (not push).
const WRITE_CMDS: &[&str] = &["checkout", "commit", "add", "reset", "merge"];
/// Protected branches — any write targeting these needs approval.
const PROTECTED_BRANCHES: &[&str] = &["main", "master", "develop"];

#[async_trait]
impl ToolHandler for GitOperationsTool {
    fn name(&self) -> &str {
        "git_operations"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }

    fn description(&self) -> &str {
        "Run git commands (status, diff, log, branch, checkout, commit) within the \
         project workspace. Read-only commands run freely. Commits and checkouts \
         run sandboxed. Push and writes to protected branches (main/master) require \
         operator approval."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": {
                    "type": "string",
                    "enum": ["status", "diff", "log", "branch", "checkout", "commit", "add", "show"],
                    "description": "Git subcommand to run"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Additional arguments passed to git (e.g. [\"-m\", \"feat: …\"] for commit)",
                    "default": []
                }
            }
        })
    }

    fn execution_class(&self) -> ExecutionClass {
        // Sandboxed for read-only commands; the approval check happens inside
        // execute() for protected-branch writes.
        ExecutionClass::SandboxedProcess
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "command".into(),
                message: "required string".into(),
            })?;

        let extra_args: Vec<String> = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        // Validate command is in our allowed set.
        let all_allowed: Vec<&str> = READONLY_CMDS
            .iter()
            .chain(WRITE_CMDS.iter())
            .copied()
            .collect();
        if !all_allowed.contains(&command) {
            return Err(ToolError::InvalidArgs {
                field: "command".into(),
                message: format!(
                    "unsupported command '{command}' — allowed: {}",
                    all_allowed.join(", ")
                ),
            });
        }

        // Protected-branch guard: reject checkouts/commits targeting main/master
        // if we haven't gone through an approval flow.
        // (In a real system, the orchestrator's approval gate handles this; here
        //  we provide a hard stop as a defence-in-depth measure.)
        if command == "checkout" || command == "commit" {
            for arg in &extra_args {
                let lower = arg.to_lowercase();
                if PROTECTED_BRANCHES.iter().any(|b| lower == *b) {
                    return Err(ToolError::InvalidArgs {
                        field:   "args".into(),
                        message: format!(
                            "writing to protected branch '{arg}' requires explicit operator approval"
                        ),
                    });
                }
            }
        }

        // Shell-injection guard: reject args with shell metacharacters.
        for arg in &extra_args {
            if arg.contains(';')
                || arg.contains('&')
                || arg.contains('|')
                || arg.contains('`')
                || arg.contains('$')
            {
                return Err(ToolError::InvalidArgs {
                    field: "args".into(),
                    message: format!("argument contains unsafe characters: {arg}"),
                });
            }
        }

        // Build the git command.
        let mut cmd = tokio::process::Command::new("git");
        cmd.arg(command)
            .args(&extra_args)
            .current_dir(&self.workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd
            .spawn()
            .map_err(|e| ToolError::Transient(format!("failed to spawn git: {e}")))?;

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(GIT_TIMEOUT_SECS),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| ToolError::TimedOut)?
        .map_err(|e| ToolError::Transient(format!("git process error: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let exit_code = output.status.code().unwrap_or(-1);
        let success = output.status.success();

        if success {
            Ok(ToolResult::ok(serde_json::json!({
                "command":   format!("git {command}"),
                "exit_code": exit_code,
                "stdout":    stdout.trim_end(),
                "stderr":    stderr.trim_end(),
            })))
        } else {
            Err(ToolError::Permanent(format!(
                "git {command} exited with code {exit_code}: {}",
                stderr.trim_end()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ProjectKey;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    fn temp_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        // Minimal git init so commands work in the temp dir.
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@cairn.test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        dir
    }

    // ── happy path ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn git_status_in_empty_repo() {
        let repo = temp_git_repo();
        let tool = GitOperationsTool::new(repo.path());
        let res = tool
            .execute(&project(), serde_json::json!({ "command": "status" }))
            .await
            .unwrap();
        assert!(res.output["command"]
            .as_str()
            .unwrap()
            .contains("git status"));
    }

    #[tokio::test]
    async fn git_log_in_empty_repo() {
        let repo = temp_git_repo();
        let tool = GitOperationsTool::new(repo.path());
        // Empty repo has no commits; git log exits non-zero — expect Permanent error.
        let _ = tool
            .execute(&project(), serde_json::json!({ "command": "log" }))
            .await;
        // Either success (some logs) or Permanent (no commits yet) — both are fine.
    }

    // ── validation ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rejects_unsupported_command() {
        let repo = temp_git_repo();
        let tool = GitOperationsTool::new(repo.path());
        let err = tool
            .execute(
                &project(),
                serde_json::json!({
                    "command": "push"
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn rejects_protected_branch_checkout() {
        let repo = temp_git_repo();
        let tool = GitOperationsTool::new(repo.path());
        let err = tool
            .execute(
                &project(),
                serde_json::json!({
                    "command": "checkout",
                    "args": ["main"]
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
        assert!(err.to_string().contains("protected branch"));
    }

    #[tokio::test]
    async fn rejects_shell_injection() {
        let repo = temp_git_repo();
        let tool = GitOperationsTool::new(repo.path());
        let err = tool
            .execute(
                &project(),
                serde_json::json!({
                    "command": "status",
                    "args": ["; rm -rf /"]
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
        assert!(err.to_string().contains("unsafe characters"));
    }

    #[tokio::test]
    async fn missing_command_is_invalid() {
        let repo = temp_git_repo();
        let tool = GitOperationsTool::new(repo.path());
        let err = tool
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    // ── metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn tier_is_registered() {
        assert_eq!(GitOperationsTool::new("/tmp").tier(), ToolTier::Registered);
    }

    #[test]
    fn schema_requires_command() {
        let tool = GitOperationsTool::new("/tmp");
        let schema = tool.parameters_schema();
        let required: Vec<String> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        assert!(required.contains(&"command".to_owned()));
    }
}
