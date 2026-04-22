//! harness-glob → cairn: `glob`.

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use cairn_tools::builtins::{
    PermissionLevel, ToolCategory, ToolContext, ToolEffect, ToolError, ToolResult,
};
use harness_core::{PermissionHook, PermissionPolicy};
use harness_glob::{glob, GlobResult, GlobSessionConfig, GLOB_TOOL_NAME};
use serde_json::{json, Value};

use crate::adapter::HarnessTool;
use crate::error::map_harness;
use crate::sensitive::default_sensitive_patterns;

pub struct HarnessGlob;

#[async_trait]
impl HarnessTool for HarnessGlob {
    type Session = GlobSessionConfig;
    type Result = GlobResult;

    fn name() -> &'static str {
        GLOB_TOOL_NAME
    }
    fn description() -> &'static str {
        "Gitignore-aware pathname glob with bash-glob semantics."
    }
    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern":    { "type": "string", "description": "Glob pattern." },
                "path":       { "type": "string", "description": "Subtree to walk." },
                "head_limit": { "type": "integer" },
                "offset":     { "type": "integer" }
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

    fn build_session(
        ctx: &ToolContext,
        _project: &ProjectKey,
        hook: PermissionHook,
    ) -> Self::Session {
        let cwd = ctx.working_dir.to_string_lossy().into_owned();
        let perms = PermissionPolicy {
            roots: vec![cwd.clone()],
            sensitive_patterns: default_sensitive_patterns(),
            hook: Some(hook),
            bypass_workspace_guard: false,
        };
        GlobSessionConfig::new(cwd, perms)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        glob(args, session).await
    }

    fn result_to_tool_result(result: Self::Result) -> Result<ToolResult, ToolError> {
        match result {
            GlobResult::Paths(p) => Ok(ToolResult::ok(json!({
                "kind": "paths",
                "output": p.output,
                "paths": p.paths,
                "meta": p.meta,
            }))),
            GlobResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}
