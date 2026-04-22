//! harness-write → cairn: `write`, `edit`, `multi_edit`.
//!
//! All three tools share a single process-global `InMemoryLedger` so
//! `NOT_READ_THIS_SESSION` enforcement is consistent across the triple.
//! Future work: scope the ledger to a session via `ToolContext`.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use cairn_tools::builtins::{
    PermissionLevel, ToolCategory, ToolContext, ToolEffect, ToolError, ToolResult,
};
use harness_core::{PermissionHook, PermissionPolicy};
use harness_write::{
    edit, multi_edit, write, EditResult, InMemoryLedger, Ledger, LedgerEntry, MultiEditResult,
    WriteResult, WriteSessionConfig, EDIT_TOOL_NAME, MULTIEDIT_TOOL_NAME, WRITE_TOOL_NAME,
};
use once_cell::sync::Lazy;
use serde_json::{json, Value};

use crate::adapter::HarnessTool;
use crate::error::map_harness;
use crate::sensitive::default_sensitive_patterns;

static GLOBAL_LEDGER: Lazy<Arc<dyn Ledger>> =
    Lazy::new(|| Arc::new(InMemoryLedger::default()) as Arc<dyn Ledger>);

/// Record a successful read in the shared write-tool ledger.
///
/// Bridges `harness-read` (which does not touch the ledger) to
/// `harness-write` (which enforces `NOT_READ_THIS_SESSION`). Called by the
/// read adapter on every successful text-read.
pub(crate) fn record_read_in_global_ledger(
    path: &str,
    sha256: &str,
    mtime_ms: u64,
    size_bytes: u64,
) {
    GLOBAL_LEDGER.record(LedgerEntry {
        path: path.to_owned(),
        sha256: sha256.to_owned(),
        mtime_ms,
        size_bytes,
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    });
}

fn build_write_session(ctx: &ToolContext, hook: PermissionHook) -> WriteSessionConfig {
    let cwd = ctx.working_dir.to_string_lossy().into_owned();
    let perms = PermissionPolicy {
        roots: vec![cwd.clone()],
        sensitive_patterns: default_sensitive_patterns(),
        hook: Some(hook),
        bypass_workspace_guard: false,
    };
    WriteSessionConfig::new(cwd, perms, GLOBAL_LEDGER.clone())
}

// ── write ────────────────────────────────────────────────────────────────────

pub struct HarnessWrite;

#[async_trait]
impl HarnessTool for HarnessWrite {
    type Session = WriteSessionConfig;
    type Result = WriteResult;

    fn name() -> &'static str {
        WRITE_TOOL_NAME
    }
    fn description() -> &'static str {
        "Atomic file write via temp-file + rename. Enforces read-before-write."
    }
    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "required": ["path", "content"],
            "properties": {
                "path": { "type": "string" },
                "content":   { "type": "string" }
            }
        })
    }
    fn execution_class() -> ExecutionClass {
        ExecutionClass::Sensitive
    }
    fn permission_level() -> PermissionLevel {
        PermissionLevel::Write
    }
    fn category() -> ToolCategory {
        ToolCategory::FileSystem
    }
    fn tool_effect() -> ToolEffect {
        ToolEffect::External
    }
    fn retry_safety() -> RetrySafety {
        RetrySafety::DangerousPause
    }

    fn build_session(ctx: &ToolContext, _project: &ProjectKey, hook: PermissionHook) -> Self::Session {
        build_write_session(ctx, hook)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        write(args, session).await
    }

    fn result_to_tool_result(result: Self::Result) -> Result<ToolResult, ToolError> {
        match result {
            WriteResult::Text(t) => Ok(ToolResult::ok(json!({
                "kind": "text",
                "output": t.output,
                "meta": t.meta,
            }))),
            WriteResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}

// ── edit ─────────────────────────────────────────────────────────────────────

pub struct HarnessEdit;

#[async_trait]
impl HarnessTool for HarnessEdit {
    type Session = WriteSessionConfig;
    type Result = EditResult;

    fn name() -> &'static str {
        EDIT_TOOL_NAME
    }
    fn description() -> &'static str {
        "Exact string-replace edit. Errors on OLD_STRING_NOT_FOUND / OLD_STRING_NOT_UNIQUE."
    }
    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "required": ["path", "old_string", "new_string"],
            "properties": {
                "path":   { "type": "string" },
                "old_string":  { "type": "string" },
                "new_string":  { "type": "string" },
                "replace_all": { "type": "boolean", "default": false },
                "dry_run":     { "type": "boolean", "default": false }
            }
        })
    }
    fn execution_class() -> ExecutionClass {
        ExecutionClass::Sensitive
    }
    fn permission_level() -> PermissionLevel {
        PermissionLevel::Write
    }
    fn category() -> ToolCategory {
        ToolCategory::FileSystem
    }
    fn tool_effect() -> ToolEffect {
        ToolEffect::External
    }
    fn retry_safety() -> RetrySafety {
        RetrySafety::DangerousPause
    }

    fn build_session(ctx: &ToolContext, _project: &ProjectKey, hook: PermissionHook) -> Self::Session {
        build_write_session(ctx, hook)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        edit(args, session).await
    }

    fn result_to_tool_result(result: Self::Result) -> Result<ToolResult, ToolError> {
        match result {
            EditResult::Text(t) => Ok(ToolResult::ok(json!({
                "kind": "text",
                "output": t.output,
                "meta": t.meta,
            }))),
            EditResult::Preview(p) => Ok(ToolResult::ok(json!({
                "kind": "preview",
                "output": p.output,
                "diff": p.diff,
                "meta": p.meta,
            }))),
            EditResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}

// ── multi_edit ───────────────────────────────────────────────────────────────

pub struct HarnessMultiEdit;

#[async_trait]
impl HarnessTool for HarnessMultiEdit {
    type Session = WriteSessionConfig;
    type Result = MultiEditResult;

    fn name() -> &'static str {
        MULTIEDIT_TOOL_NAME
    }
    fn description() -> &'static str {
        "Apply a sequence of edits atomically with rollback on failure."
    }
    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "required": ["path", "edits"],
            "properties": {
                "path": { "type": "string" },
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["old_string", "new_string"],
                        "properties": {
                            "old_string":  { "type": "string" },
                            "new_string":  { "type": "string" },
                            "replace_all": { "type": "boolean", "default": false }
                        }
                    }
                },
                "dry_run": { "type": "boolean", "default": false }
            }
        })
    }
    fn execution_class() -> ExecutionClass {
        ExecutionClass::Sensitive
    }
    fn permission_level() -> PermissionLevel {
        PermissionLevel::Write
    }
    fn category() -> ToolCategory {
        ToolCategory::FileSystem
    }
    fn tool_effect() -> ToolEffect {
        ToolEffect::External
    }
    fn retry_safety() -> RetrySafety {
        RetrySafety::DangerousPause
    }

    fn build_session(ctx: &ToolContext, _project: &ProjectKey, hook: PermissionHook) -> Self::Session {
        build_write_session(ctx, hook)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        multi_edit(args, session).await
    }

    fn result_to_tool_result(result: Self::Result) -> Result<ToolResult, ToolError> {
        match result {
            MultiEditResult::Text(t) => Ok(ToolResult::ok(json!({
                "kind": "text",
                "output": t.output,
                "meta": t.meta,
            }))),
            MultiEditResult::Preview(p) => Ok(ToolResult::ok(json!({
                "kind": "preview",
                "output": p.output,
                "diff": p.diff,
                "meta": p.meta,
            }))),
            MultiEditResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}
