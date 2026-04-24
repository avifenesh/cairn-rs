//! Helpers for deriving an [`ApprovalMatchPolicy`] from a tool call.
//!
//! The execute phase needs to pin a match policy onto every
//! `ToolCallProposed` event so that, when an operator elects
//! `ApprovalScope::Session` for a given decision, the session allow
//! registry can widen to *future* calls the operator implicitly sanctioned.
//!
//! Policy selection is convention-driven:
//!
//! | Tool shape | Tool effect | Path arg | Derived policy |
//! |---|---|---|---|
//! | Read-ish tool | `Observational` | inside project root | `ProjectScopedPath { project_root }` |
//! | Read-ish tool | `Observational` | outside project root | `ExactPath { path }` |
//! | Everything else | `Internal`/`External` | any | `Exact` |
//!
//! `Exact` is the safe default: it only auto-approves calls whose
//! canonical argument payload is byte-identical to what the operator
//! originally green-lit. Widening only happens for read-shaped tools
//! because mutation scopes are much higher blast radius — the operator
//! can still pick `Once` or an explicit `Session` scope for writes.

use cairn_domain::{decisions::ToolEffect, ApprovalMatchPolicy};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// Lexically canonicalise a path (no filesystem access). Mirrors the
/// canonicalisation rule used by
/// `cairn_runtime::tool_call_approvals::canonicalise` so policies
/// derived here stay consistent with rule-matching at evaluation time.
fn canonicalise(p: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for c in Path::new(p).components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop = matches!(
                    out.components().next_back(),
                    Some(Component::Normal(_)) | Some(Component::CurDir)
                );
                if can_pop {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Component-wise containment check. `/workspaces/cairn2` must not match
/// a root of `/workspaces/cairn` — see the `ApprovalMatchPolicy::ProjectScopedPath`
/// docstring for the full rationale.
fn is_under(candidate: &Path, root: &Path) -> bool {
    let mut ci = candidate.components();
    let mut ri = root.components();
    loop {
        match (ci.next(), ri.next()) {
            (Some(c), Some(r)) if c == r => continue,
            (_, Some(_)) => return false,
            (_, None) => return true,
        }
    }
}

/// Classify a tool's effect as "read-shaped" for the purposes of
/// widening a session-scope approval to path-based matches.
///
/// Only `Observational` tools are considered read-shaped: they read cairn
/// or external state but perform no mutations. `Internal` (writes to
/// cairn-owned state) and `External` (shell / HTTP) fall through to
/// `Exact`, which is the safer default because a "session-approved"
/// write should not silently widen to the whole workspace.
fn is_read_shaped(effect: ToolEffect) -> bool {
    matches!(effect, ToolEffect::Observational)
}

/// Derive the match policy the execute phase should stamp onto a
/// `ToolCallProposed` event.
///
/// * `effect` — tool effect (from the tool registry handler).
/// * `tool_args` — the args the LLM actually supplied.
/// * `project_root` — canonical absolute path of the project working dir
///   (usually `OrchestrationContext.working_dir`). `None` means "no
///   project scoping available" — the function falls back to `Exact`.
///
/// Selection rules are as in the module docstring.
pub fn derive_match_policy(
    effect: ToolEffect,
    tool_args: &Value,
    project_root: Option<&Path>,
) -> ApprovalMatchPolicy {
    if !is_read_shaped(effect) {
        return ApprovalMatchPolicy::Exact;
    }

    // Look up a top-level `"path"` arg. Tools that pass paths under a
    // different key get `Exact` (conservative) — adding more keys here
    // later is an additive change.
    let Some(path_str) = tool_args.get("path").and_then(Value::as_str) else {
        return ApprovalMatchPolicy::Exact;
    };

    let canon_candidate = canonicalise(path_str);

    if let Some(root) = project_root {
        let canon_root = canonicalise(&root.display().to_string());
        if is_under(&canon_candidate, &canon_root) {
            return ApprovalMatchPolicy::ProjectScopedPath {
                project_root: canon_root.display().to_string(),
            };
        }
    }

    ApprovalMatchPolicy::ExactPath {
        path: canon_candidate.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mutating_tool_gets_exact() {
        let policy = derive_match_policy(
            ToolEffect::Internal,
            &json!({"path": "/a/b"}),
            Some(Path::new("/a")),
        );
        assert!(matches!(policy, ApprovalMatchPolicy::Exact));
    }

    #[test]
    fn read_inside_project_widens_to_project_scope() {
        let policy = derive_match_policy(
            ToolEffect::Observational,
            &json!({"path": "/workspaces/cairn/src/lib.rs"}),
            Some(Path::new("/workspaces/cairn")),
        );
        match policy {
            ApprovalMatchPolicy::ProjectScopedPath { project_root } => {
                assert_eq!(project_root, "/workspaces/cairn");
            }
            other => panic!("expected project-scoped, got {other:?}"),
        }
    }

    #[test]
    fn read_outside_project_gets_exact_path() {
        let policy = derive_match_policy(
            ToolEffect::Observational,
            &json!({"path": "/etc/passwd"}),
            Some(Path::new("/workspaces/cairn")),
        );
        match policy {
            ApprovalMatchPolicy::ExactPath { path } => assert_eq!(path, "/etc/passwd"),
            other => panic!("expected exact path, got {other:?}"),
        }
    }

    #[test]
    fn read_without_path_arg_gets_exact() {
        let policy = derive_match_policy(
            ToolEffect::Observational,
            &json!({"q": "hello"}),
            Some(Path::new("/a")),
        );
        assert!(matches!(policy, ApprovalMatchPolicy::Exact));
    }

    #[test]
    fn read_without_project_root_gets_exact_path() {
        let policy = derive_match_policy(ToolEffect::Observational, &json!({"path": "/x/y"}), None);
        match policy {
            ApprovalMatchPolicy::ExactPath { path } => assert_eq!(path, "/x/y"),
            other => panic!("expected exact path, got {other:?}"),
        }
    }

    #[test]
    fn sibling_root_does_not_widen() {
        // /workspaces/cairn2 must not widen to /workspaces/cairn.
        let policy = derive_match_policy(
            ToolEffect::Observational,
            &json!({"path": "/workspaces/cairn2/a"}),
            Some(Path::new("/workspaces/cairn")),
        );
        match policy {
            ApprovalMatchPolicy::ExactPath { path } => assert_eq!(path, "/workspaces/cairn2/a"),
            other => panic!("expected exact path (sibling root not subset), got {other:?}"),
        }
    }
}
