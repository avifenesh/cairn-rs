//! harness-bash → cairn: `bash`, `bash_output`, `bash_kill`.

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use cairn_tools::builtins::{
    PermissionLevel, ToolCategory, ToolContext, ToolEffect, ToolError, ToolResult,
};
use harness_bash::{
    bash, bash_kill, bash_output, default_executor, BashKillResult, BashOutputResult,
    BashPermissionPolicy, BashResult, BashSessionConfig, BASH_KILL_TOOL_NAME,
    BASH_OUTPUT_TOOL_NAME, BASH_TOOL_NAME,
};
use harness_core::{PermissionHook, PermissionPolicy};
use serde_json::{json, Value};

use crate::adapter::HarnessTool;
use crate::error::map_harness;
use crate::sensitive::default_sensitive_patterns;

fn build_bash_session(ctx: &ToolContext, hook: PermissionHook) -> BashSessionConfig {
    let cwd = ctx.working_dir.to_string_lossy().into_owned();
    let inner = PermissionPolicy {
        roots: vec![cwd.clone()],
        sensitive_patterns: default_sensitive_patterns(),
        hook: Some(hook),
        bypass_workspace_guard: false,
    };
    let perms = BashPermissionPolicy::new(inner);
    BashSessionConfig::new(cwd, perms, default_executor())
}

// ── bash ─────────────────────────────────────────────────────────────────────

pub struct HarnessBash;

#[async_trait]
impl HarnessTool for HarnessBash {
    type Session = BashSessionConfig;
    type Result = BashResult;

    fn name() -> &'static str {
        BASH_TOOL_NAME
    }
    fn description() -> &'static str {
        "Run a shell command in a bash subprocess. SENSITIVE — requires operator approval."
    }
    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command":    { "type": "string", "description": "Shell command to execute." },
                "cwd":        { "type": "string", "description": "Working directory override." },
                "timeout_ms": { "type": "integer", "description": "Inactivity timeout in ms." },
                "description":{ "type": "string", "description": "Human-readable job label." },
                "background": { "type": "boolean", "description": "Run as a background job." },
                "env":        { "type": "object", "description": "Extra env vars.",
                                "additionalProperties": { "type": "string" } }
            }
        })
    }
    fn execution_class() -> ExecutionClass {
        ExecutionClass::Sensitive
    }
    fn permission_level() -> PermissionLevel {
        PermissionLevel::Execute
    }
    fn category() -> ToolCategory {
        ToolCategory::Shell
    }
    fn tool_effect() -> ToolEffect {
        ToolEffect::External
    }
    fn retry_safety() -> RetrySafety {
        RetrySafety::DangerousPause
    }

    fn build_session(
        ctx: &ToolContext,
        _project: &ProjectKey,
        hook: PermissionHook,
    ) -> Self::Session {
        build_bash_session(ctx, hook)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        bash(args, session).await
    }

    fn result_to_tool_result(result: Self::Result) -> Result<ToolResult, ToolError> {
        match result {
            BashResult::Ok(ok) => {
                let truncated = ok.byte_cap;
                let v = json!({
                    "kind": "ok",
                    "output": ok.output,
                    "exit_code": ok.exit_code,
                    "stdout": ok.stdout,
                    "stderr": ok.stderr,
                    "duration_ms": ok.duration_ms,
                    "log_path": ok.log_path,
                    "byte_cap": ok.byte_cap,
                });
                Ok(if truncated {
                    ToolResult::truncated(v)
                } else {
                    ToolResult::ok(v)
                })
            }
            BashResult::NonzeroExit(nz) => {
                let truncated = nz.byte_cap;
                let v = json!({
                    "kind": "nonzero_exit",
                    "output": nz.output,
                    "exit_code": nz.exit_code,
                    "stdout": nz.stdout,
                    "stderr": nz.stderr,
                    "duration_ms": nz.duration_ms,
                    "log_path": nz.log_path,
                    "byte_cap": nz.byte_cap,
                });
                Ok(if truncated {
                    ToolResult::truncated(v)
                } else {
                    ToolResult::ok(v)
                })
            }
            BashResult::Timeout(_t) => Err(ToolError::TimedOut),
            BashResult::BackgroundStarted(bg) => Ok(ToolResult::ok(json!({
                "kind": "background_started",
                "output": bg.output,
                "job_id": bg.job_id,
            }))),
            BashResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}

// ── bash_output ──────────────────────────────────────────────────────────────

pub struct HarnessBashOutput;

#[async_trait]
impl HarnessTool for HarnessBashOutput {
    type Session = BashSessionConfig;
    type Result = BashOutputResult;

    fn name() -> &'static str {
        BASH_OUTPUT_TOOL_NAME
    }
    fn description() -> &'static str {
        "Poll a backgrounded bash job's output since a given byte offset."
    }
    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "required": ["job_id"],
            "properties": {
                "job_id":     { "type": "string", "description": "ID returned by background bash." },
                "since_byte": { "type": "integer", "description": "Byte offset to resume from." },
                "head_limit": { "type": "integer", "description": "Max lines to return." }
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
        ToolCategory::Shell
    }
    fn tool_effect() -> ToolEffect {
        ToolEffect::Observational
    }

    fn build_session(
        ctx: &ToolContext,
        _project: &ProjectKey,
        hook: PermissionHook,
    ) -> Self::Session {
        build_bash_session(ctx, hook)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        bash_output(args, session).await
    }

    fn result_to_tool_result(result: Self::Result) -> Result<ToolResult, ToolError> {
        match result {
            BashOutputResult::Output {
                output,
                running,
                exit_code,
                stdout,
                stderr,
                total_bytes_stdout,
                total_bytes_stderr,
                next_since_byte,
            } => Ok(ToolResult::ok(json!({
                "kind": "output",
                "output": output,
                "running": running,
                "exit_code": exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "total_bytes_stdout": total_bytes_stdout,
                "total_bytes_stderr": total_bytes_stderr,
                "next_since_byte": next_since_byte,
            }))),
            BashOutputResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}

// ── bash_kill ────────────────────────────────────────────────────────────────

pub struct HarnessBashKill;

#[async_trait]
impl HarnessTool for HarnessBashKill {
    type Session = BashSessionConfig;
    type Result = BashKillResult;

    fn name() -> &'static str {
        BASH_KILL_TOOL_NAME
    }
    fn description() -> &'static str {
        "Send a termination signal to a backgrounded bash job."
    }
    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "required": ["job_id"],
            "properties": {
                "job_id": { "type": "string", "description": "ID returned by background bash." },
                "signal": { "type": "string", "description": "Signal name (default SIGTERM)." }
            }
        })
    }
    fn execution_class() -> ExecutionClass {
        ExecutionClass::Sensitive
    }
    fn permission_level() -> PermissionLevel {
        PermissionLevel::Execute
    }
    fn category() -> ToolCategory {
        ToolCategory::Shell
    }
    fn tool_effect() -> ToolEffect {
        ToolEffect::External
    }

    fn build_session(
        ctx: &ToolContext,
        _project: &ProjectKey,
        hook: PermissionHook,
    ) -> Self::Session {
        build_bash_session(ctx, hook)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        bash_kill(args, session).await
    }

    fn result_to_tool_result(result: Self::Result) -> Result<ToolResult, ToolError> {
        match result {
            BashKillResult::Killed {
                output,
                job_id,
                signal,
            } => Ok(ToolResult::ok(json!({
                "kind": "killed",
                "output": output,
                "job_id": job_id,
                "signal": signal,
            }))),
            BashKillResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}
