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
//! `(tenant, workspace, project, session_id, run_id)`. Two sessions never
//! share an activated set, two projects within the same session never
//! share one either, and two runs within the same session get fresh sets
//! — activating a skill in run A does not suppress re-injection of the
//! body into run B's conversation. This matches the five-field ledger
//! key in `cairn-harness-tools::tools::write::LEDGERS`.
//!
//! When both `session_id` and `run_id` are absent (e.g. unit-test call
//! sites building `ToolContext::default()`), all such invocations within
//! the same project share one set. That is intentional for tests; real
//! execution always supplies both fields via `ToolContext::for_run`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use cairn_harness_tools::error::map_harness;
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

/// Maximum number of distinct (tenant, workspace, project, session, run)
/// keys retained in `ACTIVATED_SETS`. When the cache exceeds this, the
/// least-recently-inserted half is evicted in one pass.
///
/// 1024 fits several hundred concurrent multi-run sessions at typical
/// cairn-app workloads while keeping worst-case memory bounded (each
/// entry holds a small `HashSet<String>` behind an `Arc<Mutex<_>>`).
/// Operators with higher session churn should call `evict_session`
/// from the run-finalize path (follow-up: wire that hook explicitly).
const MAX_CACHED_SESSIONS: usize = 1024;

/// Process-wide cache of per-session activated sets.
///
/// Keyed by `(tenant, workspace, project, session_id, run_id)` so two
/// runs within the same session get independent sets and a skill
/// activated in run A does not silence re-injection in run B. Bounded
/// by `MAX_CACHED_SESSIONS` with half-eviction on overflow.
static ACTIVATED_SETS: Lazy<Mutex<HashMap<String, ActivatedSet>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Build the cache key. When both `session_id` and `run_id` are absent
/// (e.g. `ToolContext::default()` in unit tests), returns a per-call
/// unique key so session-less invocations cannot share dedupe state by
/// accident. Production code paths always supply at least one field.
fn session_key(ctx: &ToolContext, project: &ProjectKey) -> String {
    match (ctx.session_id.as_deref(), ctx.run_id.as_deref()) {
        (None, None) => {
            // Per-call unique key; callers that want shared dedupe must
            // supply session_id or run_id.
            use std::sync::atomic::{AtomicU64, Ordering};
            static NONCE: AtomicU64 = AtomicU64::new(0);
            let n = NONCE.fetch_add(1, Ordering::Relaxed);
            format!(
                "{}/{}/{}/__anon__/{}",
                project.tenant_id, project.workspace_id, project.project_id, n,
            )
        }
        (s, r) => format!(
            "{}/{}/{}/{}/{}",
            project.tenant_id,
            project.workspace_id,
            project.project_id,
            s.unwrap_or(""),
            r.unwrap_or(""),
        ),
    }
}

/// Lookup or create the `ActivatedSet` for this (project, session, run)
/// tuple. On cache overflow, evicts half the entries (oldest insertion
/// order is not tracked; we rely on HashMap's iteration order which is
/// randomised per run — fine for a pressure-relief valve, not an LRU).
///
/// On mutex poisoning (a prior panic while the lock was held) we recover
/// the inner map instead of propagating — tool calls should not fail
/// because of an unrelated panic in a different task.
fn activated_set_for(ctx: &ToolContext, project: &ProjectKey) -> ActivatedSet {
    let key = session_key(ctx, project);
    let mut guard = ACTIVATED_SETS.lock().unwrap_or_else(|e| e.into_inner());
    if guard.len() >= MAX_CACHED_SESSIONS && !guard.contains_key(&key) {
        // Pressure relief: drop half the keys. Cheaper than tracking an
        // LRU linked list and acceptable for a cache that only affects
        // dedupe (worst case: a skill re-injects its body once).
        let victims: Vec<String> = guard
            .keys()
            .take(MAX_CACHED_SESSIONS / 2)
            .cloned()
            .collect();
        for v in victims {
            guard.remove(&v);
        }
    }
    guard.entry(key).or_default().clone()
}

/// Drop the cached `ActivatedSet` for a (project, session, run) tuple.
///
/// Call this from the orchestrator's run-finalize path so long-running
/// cairn-app processes do not accumulate per-run state indefinitely.
/// Safe to call with either field missing; on a cache miss it is a no-op.
pub fn evict_session(project: &ProjectKey, session_id: Option<&str>, run_id: Option<&str>) {
    let key = format!(
        "{}/{}/{}/{}/{}",
        project.tenant_id,
        project.workspace_id,
        project.project_id,
        session_id.unwrap_or(""),
        run_id.unwrap_or(""),
    );
    let mut guard = ACTIVATED_SETS.lock().unwrap_or_else(|e| e.into_inner());
    guard.remove(&key);
}

/// Test-only: clear the cache between test cases so dedupe state does not
/// leak across tests that reuse session IDs.
///
/// Only compiled when the `test-utils` cargo feature is enabled.
#[cfg(feature = "test-utils")]
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
///
/// Called internally by `build_session`; publicly re-exported from the
/// crate root only under the `test-utils` feature for parity testing.
#[doc(hidden)]
pub fn skill_roots_for(ctx: &ToolContext) -> Vec<String> {
    let cwd: PathBuf = ctx.working_dir.clone();
    vec![
        cwd.join(".cairn")
            .join("skills")
            .to_string_lossy()
            .into_owned(),
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
        // `SupervisedProcess` is the non-sandboxed, non-sensitive default —
        // the domain enum exposes only `SupervisedProcess | SandboxedProcess
        // | Sensitive`. Skill activation reads a SKILL.md from an authored
        // workspace root and injects prose; no subprocess, no network, no
        // operator approval required before dispatch.
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
            SkillResult::Ok(ok) => {
                // `output` is the harness-formatted rendering that wraps
                // the skill body in `<skill>…</skill>` tags — it is what
                // the LLM reads. We do NOT echo `body` / `frontmatter` /
                // `bytes` separately; duplicating that content wastes
                // tokens on every activation. `name` lets callers pattern-
                // match on which skill activated, `resources` tells the
                // model what supporting files exist, `dir` lets it resolve
                // resource paths via a follow-up read tool call.
                Ok(ToolResult::ok(json!({
                    "kind": "ok",
                    "name": ok.name,
                    "dir": ok.dir,
                    "output": ok.output,
                    "resources": ok.resources,
                })))
            }
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
            SkillResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}
