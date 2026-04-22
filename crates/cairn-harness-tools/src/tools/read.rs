//! harness-read → cairn: `read`.

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use cairn_tools::builtins::{
    PermissionLevel, ToolCategory, ToolContext, ToolEffect, ToolError, ToolResult,
};
use harness_core::{PermissionHook, PermissionPolicy};
use harness_read::{read, ReadResult, ReadSessionConfig, READ_TOOL_NAME};
use serde_json::{json, Value};

use crate::adapter::HarnessTool;
use crate::error::map_harness;
use crate::sensitive::default_sensitive_patterns;
use crate::tools::write::record_read_in_global_ledger;

pub struct HarnessRead;

#[async_trait]
impl HarnessTool for HarnessRead {
    type Session = ReadSessionConfig;
    type Result = ReadResult;

    fn name() -> &'static str {
        READ_TOOL_NAME
    }
    fn description() -> &'static str {
        "Read a file from the local filesystem, with 1-indexed offset/limit pagination."
    }
    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string", "description": "Absolute or workspace-relative path." },
                "offset":    { "type": "integer", "description": "1-indexed starting line." },
                "limit":     { "type": "integer", "description": "Max lines to return." }
            }
        })
    }
    fn execution_class() -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }
    fn permission_level() -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category() -> ToolCategory {
        ToolCategory::FileSystem
    }
    fn tool_effect() -> ToolEffect {
        ToolEffect::Observational
    }
    fn retry_safety() -> RetrySafety {
        RetrySafety::IdempotentSafe
    }

    fn build_session(ctx: &ToolContext, _project: &ProjectKey, hook: PermissionHook) -> Self::Session {
        let cwd = ctx.working_dir.to_string_lossy().into_owned();
        let perms = PermissionPolicy {
            roots: vec![cwd.clone()],
            sensitive_patterns: default_sensitive_patterns(),
            hook: Some(hook),
            bypass_workspace_guard: false,
        };
        ReadSessionConfig::new(cwd, perms)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        read(args, session).await
    }

    fn result_to_tool_result(result: Self::Result) -> Result<ToolResult, ToolError> {
        match result {
            ReadResult::Text(t) => {
                // Populate the write-tool ledger so a subsequent edit / multi_edit
                // passes the NOT_READ_THIS_SESSION gate. harness-read doesn't
                // touch the write ledger, so the adapter bridges them.
                record_read_in_global_ledger(
                    &t.meta.path,
                    &t.meta.sha256,
                    t.meta.mtime_ms,
                    t.meta.size_bytes,
                );
                Ok(ToolResult::ok(json!({
                    "kind": "text",
                    "output": t.output,
                    "meta": t.meta,
                })))
            }
            ReadResult::Directory(d) => Ok(ToolResult::ok(json!({
                "kind": "directory",
                "output": d.output,
                "meta": d.meta,
            }))),
            ReadResult::Attachment(a) => Ok(ToolResult::ok(json!({
                "kind": "attachment",
                "output": a.output,
                "attachments": a.attachments,
                "meta": a.meta,
            }))),
            ReadResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}
