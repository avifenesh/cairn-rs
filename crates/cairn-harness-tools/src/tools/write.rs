//! harness-write → cairn: `write`, `edit`, `multi_edit`.
//!
//! The upstream `NOT_READ_THIS_SESSION` guard is enforced against a
//! per-session `InMemoryLedger`. Ledgers are keyed by
//! `(ToolContext.session_id, ToolContext.run_id)` so a read in one run
//! never satisfies the guard for a different run or a different tenant.
//! Contexts without `session_id` fall back to the project key supplied at
//! tool-exec time so unit-test call sites still function.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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

/// Per-session write-ledger cache.
///
/// Keyed by `(tenant_id, workspace_id, project_id, session_id.unwrap_or(""))`
/// so cross-tenant + cross-run ledger pollution is impossible.
static LEDGERS: Lazy<Mutex<HashMap<String, Arc<dyn Ledger>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn ledger_key(ctx: &ToolContext, project: &ProjectKey) -> String {
    format!(
        "{}/{}/{}/{}/{}",
        project.tenant_id,
        project.workspace_id,
        project.project_id,
        ctx.session_id.as_deref().unwrap_or(""),
        ctx.run_id.as_deref().unwrap_or(""),
    )
}

/// Lookup or create the ledger for this (project, session, run) tuple.
pub(crate) fn ledger_for(ctx: &ToolContext, project: &ProjectKey) -> Arc<dyn Ledger> {
    let key = ledger_key(ctx, project);
    let mut guard = LEDGERS.lock().unwrap();
    guard
        .entry(key)
        .or_insert_with(|| Arc::new(InMemoryLedger::default()) as Arc<dyn Ledger>)
        .clone()
}

/// Record a successful read in the session's write-tool ledger.
///
/// Bridges `harness-read` (which does not touch the ledger) to
/// `harness-write` (which enforces `NOT_READ_THIS_SESSION`). Called by the
/// read adapter on every successful text-read.
pub(crate) fn record_read_in_ledger(
    ctx: &ToolContext,
    project: &ProjectKey,
    path: &str,
    sha256: &str,
    mtime_ms: u64,
    size_bytes: u64,
) {
    ledger_for(ctx, project).record(LedgerEntry {
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

fn build_write_session(
    ctx: &ToolContext,
    project: &ProjectKey,
    hook: PermissionHook,
) -> WriteSessionConfig {
    let cwd = ctx.working_dir.to_string_lossy().into_owned();
    let perms = PermissionPolicy {
        roots: vec![cwd.clone()],
        sensitive_patterns: default_sensitive_patterns(),
        hook: Some(hook),
        bypass_workspace_guard: false,
    };
    WriteSessionConfig::new(cwd, perms, ledger_for(ctx, project))
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

    fn build_session(
        ctx: &ToolContext,
        project: &ProjectKey,
        hook: PermissionHook,
    ) -> Self::Session {
        build_write_session(ctx, project, hook)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        write(args, session).await
    }

    fn result_to_tool_result(
        result: Self::Result,
        _ctx: &ToolContext,
        _project: &ProjectKey,
    ) -> Result<ToolResult, ToolError> {
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

    fn build_session(
        ctx: &ToolContext,
        project: &ProjectKey,
        hook: PermissionHook,
    ) -> Self::Session {
        build_write_session(ctx, project, hook)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        edit(args, session).await
    }

    fn result_to_tool_result(
        result: Self::Result,
        _ctx: &ToolContext,
        _project: &ProjectKey,
    ) -> Result<ToolResult, ToolError> {
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

    fn build_session(
        ctx: &ToolContext,
        project: &ProjectKey,
        hook: PermissionHook,
    ) -> Self::Session {
        build_write_session(ctx, project, hook)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        multi_edit(args, session).await
    }

    fn result_to_tool_result(
        result: Self::Result,
        _ctx: &ToolContext,
        _project: &ProjectKey,
    ) -> Result<ToolResult, ToolError> {
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
