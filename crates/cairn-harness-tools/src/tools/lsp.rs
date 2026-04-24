//! harness-lsp → cairn: `lsp`.
//!
//! Language-server operations (hover, definition, references, documentSymbol,
//! workspaceSymbol, implementation) with 1-indexed positions and
//! `server_starting` retry hints.
//!
//! # Session cache
//!
//! LSP servers are expensive to spawn (rust-analyzer can take 30s+ to index).
//! We cache one `SpawnLspClient` per cairn session keyed by
//! `(tenant, workspace, project, session_id)`. The client owns the spawned
//! server processes and their stdio pumps; a second call in the same session
//! reuses the already-warm server. Different sessions (e.g. separate runs in
//! different projects) get isolated clients so tenant boundaries stay firm.
//!
//! The cache grows unbounded over the cairn-app process lifetime. Eviction on
//! run-finalize is a follow-up, same pattern as the write-ledger cache. For
//! typical single-run lifetimes this is not urgent; for long-lived servers it
//! is worth wiring to the `SessionEnded` event. Tracked alongside #228.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use cairn_tools::builtins::{
    PermissionLevel, ToolCategory, ToolContext, ToolEffect, ToolError, ToolResult,
};
use harness_core::{PermissionHook, PermissionPolicy};
use harness_lsp::{
    lsp, LspClient, LspPermissionPolicy, LspResult, LspSessionConfig, SpawnLspClient,
    LSP_TOOL_DESCRIPTION, LSP_TOOL_NAME,
};
use once_cell::sync::Lazy;
use serde_json::{json, Value};

use crate::adapter::HarnessTool;
use crate::error::map_harness;
use crate::sensitive::default_sensitive_patterns;

/// Per-session LSP client cache.
///
/// Keyed by `(tenant_id, workspace_id, project_id, session_id)` so
/// cross-tenant + cross-session language-server processes are never shared.
/// The inner `Arc<SpawnLspClient>` owns the spawned `rust-analyzer` /
/// `gopls` / `typescript-language-server` / etc. child processes for the
/// lifetime of that session.
static CLIENTS: Lazy<Mutex<HashMap<String, Arc<SpawnLspClient>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn client_key(ctx: &ToolContext, project: &ProjectKey) -> String {
    format!(
        "{}/{}/{}/{}",
        project.tenant_id,
        project.workspace_id,
        project.project_id,
        ctx.session_id.as_deref().unwrap_or(""),
    )
}

/// Look up or spawn the cached `SpawnLspClient` for this session.
///
/// First call in a session spawns a fresh client (which lazily spawns LSP
/// processes on first operation). Subsequent calls reuse the same client,
/// so warm servers are preserved across tool invocations.
///
/// On mutex poisoning (a prior panic under the lock) we recover the inner
/// map rather than propagating; tool calls should not fail because of an
/// unrelated panic in another task.
#[doc(hidden)]
pub fn client_for(ctx: &ToolContext, project: &ProjectKey) -> Arc<SpawnLspClient> {
    let key = client_key(ctx, project);
    let mut guard = CLIENTS.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .entry(key)
        .or_insert_with(|| Arc::new(SpawnLspClient::new()))
        .clone()
}

/// Test helper: drop every cached LSP client. Calls `close_session` on each
/// so child processes exit cleanly. Exposed for adapter tests only.
#[doc(hidden)]
pub async fn __clear_client_cache_for_tests() {
    let clients: Vec<Arc<SpawnLspClient>> = {
        let mut guard = CLIENTS.lock().unwrap_or_else(|e| e.into_inner());
        guard.drain().map(|(_, v)| v).collect()
    };
    for c in clients {
        c.close_session().await;
    }
}

pub struct HarnessLsp;

#[async_trait]
impl HarnessTool for HarnessLsp {
    type Session = LspSessionConfig;
    type Result = LspResult;

    fn name() -> &'static str {
        LSP_TOOL_NAME
    }

    fn description() -> &'static str {
        LSP_TOOL_DESCRIPTION
    }

    fn parameters_schema() -> Value {
        // Mirrors harness-lsp's per-operation schema: path+line+character for
        // positional ops, path-only for documentSymbol, query-only for
        // workspaceSymbol. Positions are 1-INDEXED — matches grep/read output.
        json!({
            "type": "object",
            "required": ["operation"],
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "hover",
                        "definition",
                        "references",
                        "documentSymbol",
                        "workspaceSymbol",
                        "implementation"
                    ],
                    "description": "Which LSP operation to perform."
                },
                "path": {
                    "type": "string",
                    "description": "Absolute or workspace-relative file path. Required for every op except workspaceSymbol."
                },
                "line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-INDEXED line. Required for hover, definition, references, implementation."
                },
                "character": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-INDEXED column. Required for hover, definition, references, implementation."
                },
                "query": {
                    "type": "string",
                    "description": "Symbol-name substring. Required for workspaceSymbol."
                },
                "head_limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Max results for references / workspaceSymbol (default 200)."
                }
            }
        })
    }

    fn execution_class() -> ExecutionClass {
        // LSP spawns language-server subprocesses but only reads code; treat
        // it the same as grep — supervised but not sensitive.
        ExecutionClass::SupervisedProcess
    }

    fn permission_level() -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn category() -> ToolCategory {
        ToolCategory::FileSystem
    }

    fn tool_effect() -> ToolEffect {
        // LSP queries don't mutate the tree; Plan mode should see them.
        ToolEffect::Observational
    }

    fn retry_safety() -> RetrySafety {
        // Hover / definition / references at a given position are
        // deterministic reads of the workspace.
        RetrySafety::IdempotentSafe
    }

    fn build_session(
        ctx: &ToolContext,
        project: &ProjectKey,
        hook: PermissionHook,
    ) -> Self::Session {
        let cwd = ctx.working_dir.to_string_lossy().into_owned();
        let inner = PermissionPolicy {
            roots: vec![cwd.clone()],
            sensitive_patterns: default_sensitive_patterns(),
            hook: Some(hook),
            bypass_workspace_guard: false,
        };
        let perms = LspPermissionPolicy::new(inner);
        let client: Arc<dyn LspClient> = client_for(ctx, project);
        LspSessionConfig::new(cwd, perms, client)
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        lsp(args, session).await
    }

    fn result_to_tool_result(
        result: Self::Result,
        _ctx: &ToolContext,
        _project: &ProjectKey,
    ) -> Result<ToolResult, ToolError> {
        match result {
            LspResult::Hover(h) => Ok(ToolResult::ok(json!({
                "kind": "hover",
                "output": h.output,
                "path": h.path,
                "line": h.line,
                "character": h.character,
                "contents": h.contents,
                "is_markdown": h.is_markdown,
            }))),
            LspResult::Definition(d) => Ok(ToolResult::ok(json!({
                "kind": "definition",
                "output": d.output,
                "path": d.path,
                "line": d.line,
                "character": d.character,
                "locations": d.locations,
            }))),
            LspResult::References(r) => {
                let v = json!({
                    "kind": "references",
                    "output": r.output,
                    "path": r.path,
                    "line": r.line,
                    "character": r.character,
                    "locations": r.locations,
                    "total": r.total,
                    "truncated": r.truncated,
                });
                Ok(if r.truncated {
                    ToolResult::truncated(v)
                } else {
                    ToolResult::ok(v)
                })
            }
            LspResult::DocumentSymbol(s) => Ok(ToolResult::ok(json!({
                "kind": "documentSymbol",
                "output": s.output,
                "path": s.path,
                "symbols": s.symbols,
            }))),
            LspResult::WorkspaceSymbol(w) => {
                let v = json!({
                    "kind": "workspaceSymbol",
                    "output": w.output,
                    "query": w.query,
                    "symbols": w.symbols,
                    "total": w.total,
                    "truncated": w.truncated,
                });
                Ok(if w.truncated {
                    ToolResult::truncated(v)
                } else {
                    ToolResult::ok(v)
                })
            }
            LspResult::Implementation(i) => Ok(ToolResult::ok(json!({
                "kind": "implementation",
                "output": i.output,
                "path": i.path,
                "line": i.line,
                "character": i.character,
                "locations": i.locations,
            }))),
            LspResult::NoResults(n) => Ok(ToolResult::ok(json!({
                "kind": "no_results",
                "output": n.output,
                "operation": n.operation,
            }))),
            LspResult::ServerStarting(s) => Ok(ToolResult::ok(json!({
                "kind": "server_starting",
                "output": s.output,
                "language": s.language,
                "retry_ms": s.retry_ms,
            }))),
            LspResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}
