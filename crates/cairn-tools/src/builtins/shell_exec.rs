//! `shell_exec` built-in tool stub — sandboxed shell command execution.
//!
//! Full implementation is deferred. This stub satisfies the registry export.

use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

pub struct ShellExecTool;

impl Default for ShellExecTool { fn default() -> Self { Self } }

#[async_trait]
impl ToolHandler for ShellExecTool {
    fn name(&self) -> &str { "shell_exec" }
    fn tier(&self) -> ToolTier { ToolTier::Registered }
    fn description(&self) -> &str { "Execute a sandboxed shell command." }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({ "type": "object", "required": ["command"],
            "properties": { "command": { "type": "string" } } })
    }
    async fn execute(&self, _project: &ProjectKey, _args: Value) -> Result<ToolResult, ToolError> {
        Err(ToolError::Permanent("shell_exec not yet implemented".into()))
    }
}
