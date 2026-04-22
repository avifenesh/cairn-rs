//! harness-grep → cairn: `grep`.

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use cairn_tools::builtins::{
    PermissionLevel, ToolCategory, ToolContext, ToolEffect, ToolError, ToolResult,
};
use harness_core::{PermissionHook, PermissionPolicy};
use harness_grep::{grep, GrepResult, GrepSessionConfig, GREP_TOOL_NAME};
use serde_json::{json, Value};

use crate::adapter::HarnessTool;
use crate::error::map_harness;
use crate::sensitive::default_sensitive_patterns;

pub struct HarnessGrep;

#[async_trait]
impl HarnessTool for HarnessGrep {
    type Session = GrepSessionConfig;
    type Result = GrepResult;

    fn name() -> &'static str {
        GREP_TOOL_NAME
    }
    fn description() -> &'static str {
        "Ripgrep-backed file-content search with .gitignore-aware walking."
    }
    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern":      { "type": "string", "description": "Regex (ripgrep syntax)." },
                "path":         { "type": "string", "description": "Subtree to search." },
                "glob":         { "type": "string", "description": "Glob filter for files." },
                "type":         { "type": "string", "description": "ripgrep file-type alias." },
                "output_mode":  { "type": "string", "enum": ["files_with_matches", "content", "count"] },
                "-i":           { "type": "boolean", "description": "Case-insensitive." },
                "-n":           { "type": "boolean", "description": "Include line numbers." },
                "-A":           { "type": "integer", "description": "Lines of context after." },
                "-B":           { "type": "integer", "description": "Lines of context before." },
                "-C":           { "type": "integer", "description": "Lines of context around." },
                "multiline":    { "type": "boolean" },
                "head_limit":   { "type": "integer" },
                "offset":       { "type": "integer" }
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
        GrepSessionConfig::new(cwd, perms)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        grep(args, session).await
    }

    fn result_to_tool_result(result: Self::Result) -> Result<ToolResult, ToolError> {
        match result {
            GrepResult::FilesWithMatches(r) => Ok(ToolResult::ok(json!({
                "kind": "files_with_matches",
                "output": r.output,
                "paths": r.paths,
                "meta": r.meta,
            }))),
            GrepResult::Content(r) => {
                let truncated = r.meta.byte_cap;
                let v = json!({ "kind": "content", "output": r.output, "meta": r.meta });
                Ok(if truncated { ToolResult::truncated(v) } else { ToolResult::ok(v) })
            }
            GrepResult::Count(r) => Ok(ToolResult::ok(json!({
                "kind": "count",
                "output": r.output,
                "counts": r.counts,
                "meta": r.meta,
            }))),
            GrepResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}
