//! `HarnessSkill` — the `skill` builtin backed by `harness-skill::skill()`.
//!
//! Mirrors the pattern in `cairn-harness-tools::tools::*`: an empty type
//! implementing `HarnessTool`, wired up as `HarnessBuiltin::<HarnessSkill>::new()`
//! in cairn-app's registry.
//!
//! ## Session-scoped activation set
//!
//! `harness-skill` dedupes repeated activations of the same skill via an
//! `ActivatedSet` carried in `SkillSessionConfig`. Cairn rebuilds the
//! session config on every tool call (see `HarnessBuiltin::execute_with_context`),
//! so we cache the `ActivatedSet` in a process-wide map keyed by
//! `(tenant, workspace, project, session_id)`. Two sessions never share an
//! activated set, and two projects within the same session never share
//! one either. This matches the write-ledger cache pattern in
//! `cairn-harness-tools::tools::write::LEDGERS`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use cairn_harness_tools::HarnessTool;
use cairn_tools::builtins::{
    PermissionLevel, ToolCategory, ToolContext, ToolEffect, ToolError, ToolResult,
};
use harness_core::{PermissionHook, PermissionPolicy};
use harness_skill::{
    skill, ActivatedSet, FilesystemSkillRegistry, SkillPermissionPolicy, SkillResult,
    SkillSessionConfig, SkillTrustPolicy, SKILL_TOOL_DESCRIPTION, SKILL_TOOL_NAME,
};
use once_cell::sync::Lazy;
use serde_json::{json, Value};

use cairn_harness_tools::default_sensitive_patterns;

/// Process-wide cache of per-session activated sets.
///
/// Growth is bounded by live sessions; eviction on session finalize is a
/// follow-up (tracked alongside the write-ledger eviction TODO in
/// `cairn-harness-tools::tools::write`).
static ACTIVATED_SETS: Lazy<Mutex<HashMap<String, ActivatedSet>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn session_key(ctx: &ToolContext, project: &ProjectKey) -> String {
    format!(
        "{}/{}/{}/{}",
        project.tenant_id,
        project.workspace_id,
        project.project_id,
        ctx.session_id.as_deref().unwrap_or(""),
    )
}

/// Lookup or create the `ActivatedSet` for this (project, session) tuple.
///
/// On mutex poisoning (a prior panic while the lock was held) we recover
/// the inner map instead of propagating — tool calls should not fail
/// because of an unrelated panic in a different task.
fn activated_set_for(ctx: &ToolContext, project: &ProjectKey) -> ActivatedSet {
    let key = session_key(ctx, project);
    let mut guard = ACTIVATED_SETS.lock().unwrap_or_else(|e| e.into_inner());
    guard.entry(key).or_insert_with(ActivatedSet::new).clone()
}

/// Test-only: clear the cache between test cases so dedupe state does not
/// leak across tests that reuse session IDs.
#[doc(hidden)]
pub fn __clear_activated_sets_for_tests() {
    let mut guard = ACTIVATED_SETS.lock().unwrap_or_else(|e| e.into_inner());
    guard.clear();
}

/// Resolve skill discovery roots from the tool context's working directory.
///
/// v1 convention — mirrors the research-doc recommendation (§6.5):
/// - `<cwd>/.cairn/skills/` — project-level skills (lower index → higher
///   priority per harness-skill shadowing rules).
/// - `<cwd>/skills/` — alternate top-level layout for repos that keep
///   skills outside `.cairn/`.
///
/// Roots that do not exist are silently skipped by
/// `FilesystemSkillRegistry::discover` — no error if a project ships no
/// skills.
#[doc(hidden)]
pub fn skill_roots_for(ctx: &ToolContext) -> Vec<String> {
    let cwd: PathBuf = ctx.working_dir.clone();
    vec![
        cwd.join(".cairn").join("skills").to_string_lossy().into_owned(),
        cwd.join("skills").to_string_lossy().into_owned(),
    ]
}

/// The `skill` tool. Register via
/// `HarnessBuiltin::<HarnessSkill>::new()` in the cairn-app tool registry.
pub struct HarnessSkill;

#[async_trait]
impl HarnessTool for HarnessSkill {
    type Session = SkillSessionConfig;
    type Result = SkillResult;

    fn name() -> &'static str {
        SKILL_TOOL_NAME
    }

    fn description() -> &'static str {
        SKILL_TOOL_DESCRIPTION
    }

    fn parameters_schema() -> Value {
        // Matches `harness-skill::safe_parse_skill_params`: `name` is
        // required lowercase-kebab; `arguments` is optional, either a
        // string (for `$ARGUMENTS` / `$N` skills) or a string→string object
        // (for frontmatter-declared named arguments).
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name (lowercase-kebab-case, matches the SKILL.md parent directory)."
                },
                "arguments": {
                    "description": "Optional. String for positional skills ($ARGUMENTS / $1 / $2). Object of string→string for skills that declare named arguments in frontmatter.",
                    "oneOf": [
                        { "type": "string" },
                        {
                            "type": "object",
                            "additionalProperties": { "type": "string" }
                        }
                    ]
                }
            }
        })
    }

    fn execution_class() -> ExecutionClass {
        // Skill activation reads a file and injects prose. No sandboxed
        // process needed, no network, no side effects beyond the
        // conversation it is about to shape.
        ExecutionClass::SupervisedProcess
    }

    fn permission_level() -> PermissionLevel {
        // Reads SKILL.md from disk. Write-tool pre-approval contracts
        // remain separate; `allowed-tools` frontmatter is advisory in v1.
        PermissionLevel::ReadOnly
    }

    fn category() -> ToolCategory {
        // Skill is a discovery / meta-tool, not filesystem IO in the
        // read/write sense. `Custom` keeps it out of the `FileSystem`
        // bucket that gates path sandboxing.
        ToolCategory::Custom
    }

    fn tool_effect() -> ToolEffect {
        // Activating a skill injects prose into the conversation but does
        // not touch the outside world until a downstream tool runs. That
        // downstream call goes through its own permission gate.
        ToolEffect::Observational
    }

    fn retry_safety() -> RetrySafety {
        // Dedupe handles retries: re-activation returns `already_loaded`.
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
        let perms = SkillPermissionPolicy::new(inner);
        let registry = Arc::new(FilesystemSkillRegistry::new(skill_roots_for(ctx)));
        let trust = SkillTrustPolicy::default();
        let activated = activated_set_for(ctx, project);

        SkillSessionConfig {
            cwd,
            permissions: perms,
            registry,
            trust,
            // v1: every call is treated as model-initiated. User-initiated
            // semantics (`user_initiated: true` to bypass
            // `disable-model-invocation`) will arrive with the slash-command
            // UX in a follow-up PR.
            user_initiated: false,
            activated: Some(activated),
        }
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        skill(args, session).await
    }

    fn result_to_tool_result(
        result: Self::Result,
        _ctx: &ToolContext,
        _project: &ProjectKey,
    ) -> Result<ToolResult, ToolError> {
        match result {
            SkillResult::Ok(ok) => Ok(ToolResult::ok(json!({
                "kind": "ok",
                "output": ok.output,
                "name": ok.name,
                "dir": ok.dir,
                "body": ok.body,
                "frontmatter": ok.frontmatter,
                "resources": ok.resources,
                "bytes": ok.bytes,
            }))),
            SkillResult::AlreadyLoaded(al) => Ok(ToolResult::ok(json!({
                "kind": "already_loaded",
                "output": al.output,
                "name": al.name,
            }))),
            SkillResult::NotFound(nf) => Ok(ToolResult::ok(json!({
                "kind": "not_found",
                "output": nf.output,
                "name": nf.name,
                "siblings": nf.siblings,
            }))),
            SkillResult::Error(e) => Err(cairn_harness_tools_map_harness(e.error)),
        }
    }
}

/// Bridge `harness_core::ToolError` → cairn's `ToolError::HarnessError`.
///
/// `cairn_harness_tools::error::map_harness` is `pub(crate)` in that crate,
/// so we re-implement the three-field pass-through here. This is a handful
/// of lines; exposing it as public API in `cairn-harness-tools` would widen
/// that crate's surface unnecessarily.
fn cairn_harness_tools_map_harness(err: harness_core::ToolError) -> ToolError {
    ToolError::HarnessError {
        code: err.code,
        message: err.message,
        meta: err.meta,
    }
}
