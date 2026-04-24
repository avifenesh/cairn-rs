//! Tool-call approval service boundary.
//!
//! This is the **runtime service** for the tool-call approval flow added
//! in the BP-v2 wave (research doc `docs/research/llm-agent-approval-systems.md`).
//! It lives alongside [`crate::approvals::ApprovalService`] — the general
//! approval service keeps owning plan review (RFC 018), prompt-release
//! governance, RFC 022 trigger approvals, and the pre-existing run-level
//! pauses. The new `ToolCallApprovalService` owns only the tool-call
//! proposal / decision / retrieve-and-execute pattern.
//!
//! The two services coexist because tool-call approval has fundamentally
//! different semantics:
//!
//! * **Proposal persistence.** The proposal's full arguments payload is
//!   what the execute phase re-runs after operator approval, not a
//!   freshly re-inferred one. See the "lost proposal" bug in the research
//!   doc for why this matters.
//! * **Match-policy widening.** A single approval can widen to all future
//!   matching calls in the session (`ApprovalScope::Session`) based on an
//!   [`ApprovalMatchPolicy`]. Generic approvals are one-shot.
//! * **Amendment flow.** Operators may preview-edit arguments
//!   (`ToolCallAmended`) any number of times before resolving.
//! * **Await-decision hot path.** The execute phase blocks on a oneshot
//!   fired from the operator ops path, with a timeout that auto-rejects.
//!
//! Grafting those onto the generic trait would break existing callers and
//! muddy two very different contracts. Two traits, one per contract.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use cairn_domain::{
    ApprovalMatchPolicy, ApprovalScope, OperatorId, ProjectKey, RunId, SessionId, ToolCallId,
};
use serde_json::Value;

use crate::error::RuntimeError;

/// A proposed tool call awaiting auto-approval or operator decision.
///
/// Submitted by the orchestrator's execute phase after parsing the LLM
/// response. Field shapes mirror the [`cairn_domain::events::ToolCallProposed`]
/// wire event because this struct is the in-memory handoff from which
/// that event is built.
#[derive(Clone, Debug)]
pub struct ToolCallProposal {
    pub call_id: ToolCallId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub project: ProjectKey,
    pub tool_name: String,
    pub tool_args: Value,
    /// Short human-readable summary rendered in the operator UI.
    pub display_summary: Option<String>,
    /// How a `Session`-scoped decision would match future calls.
    pub match_policy: ApprovalMatchPolicy,
}

/// Result of submitting a proposal to the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Matched an existing session allow-rule — execute with original args.
    AutoApproved,
    /// Matched a deny path — do not execute, surface `reason` to the agent.
    AutoRejected { reason: String },
    /// No auto-match. Caller must `await_decision` for the operator.
    PendingOperator,
}

/// The operator's final decision on a proposal.
#[derive(Clone, Debug)]
pub enum OperatorDecision {
    /// Operator approved. `approved_args` are the arguments the execute
    /// phase should actually run — these are the original args, any
    /// `ToolCallAmended` edits, or an `approved_tool_args` override,
    /// whichever the operator settled on.
    Approved { approved_args: Value },
    /// Operator rejected. `reason` is surfaced to the agent verbatim.
    Rejected { reason: Option<String> },
    /// No operator decision arrived before the timeout elapsed.
    Timeout,
}

/// Proposal as it exists after an operator decision has been taken.
///
/// Returned from [`ToolCallApprovalService::retrieve_approved_proposal`]
/// and handed directly to the tool registry by the execute phase.
#[derive(Clone, Debug)]
pub struct ApprovedProposal {
    pub call_id: ToolCallId,
    pub tool_name: String,
    /// Final arguments to execute. Resolved per the `ToolCallApproved`
    /// precedence invariant (see `cairn_domain::events::ToolCallApproved`):
    ///
    /// 1. `approved_tool_args` if set on the approval event,
    /// 2. else the most recent `ToolCallAmended.new_tool_args`,
    /// 3. else the original `ToolCallProposed.tool_args`.
    pub tool_args: Value,
}

/// Minimal read interface for reconstituting proposals from the store
/// projection (e.g. after process restart or when the in-memory cache
/// has been evicted).
///
/// This trait is intentionally narrow: PR-2 of the wave lands a full
/// projection on `cairn-store`. For now, PR-3 depends only on the
/// contract — tests can supply their own stub, and PR-2 will impl this
/// on the store with a `blanket`-style impl.
#[async_trait]
pub trait ToolCallApprovalReader: Send + Sync {
    /// Load the materialised approval for a single call.
    async fn get_tool_call_approval(
        &self,
        call_id: &ToolCallId,
    ) -> Result<Option<ApprovedProposal>, RuntimeError>;
}

/// Service boundary for the tool-call approval flow.
///
/// Orchestrator side: `submit_proposal` → (if pending) `await_decision`
/// → `retrieve_approved_proposal`.
///
/// Operator side: `approve` / `reject` / `amend`.
#[async_trait]
pub trait ToolCallApprovalService: Send + Sync {
    /// Accept a proposal. Persists it, evaluates the session allow
    /// registry, and returns the decision.
    async fn submit_proposal(
        &self,
        proposal: ToolCallProposal,
    ) -> Result<ApprovalDecision, RuntimeError>;

    /// Resolve a proposal as approved. If `approved_args` is `Some`, that
    /// payload overrides the original and any prior amendments; otherwise
    /// the service re-uses whichever args are current in the cache
    /// (amended or original).
    async fn approve(
        &self,
        call_id: ToolCallId,
        operator_id: OperatorId,
        scope: ApprovalScope,
        approved_args: Option<Value>,
    ) -> Result<(), RuntimeError>;

    /// Resolve a proposal as rejected.
    async fn reject(
        &self,
        call_id: ToolCallId,
        operator_id: OperatorId,
        reason: Option<String>,
    ) -> Result<(), RuntimeError>;

    /// Amend (preview-edit) a proposal's arguments without yet resolving.
    /// Operators may amend any number of times before calling `approve`
    /// or `reject`.
    async fn amend(
        &self,
        call_id: ToolCallId,
        operator_id: OperatorId,
        new_args: Value,
    ) -> Result<(), RuntimeError>;

    /// Retrieve the final approved proposal for execution. Falls back to
    /// the store projection if the in-memory cache has been evicted.
    async fn retrieve_approved_proposal(
        &self,
        call_id: &ToolCallId,
    ) -> Result<ApprovedProposal, RuntimeError>;

    /// Block until the operator resolves the proposal or `timeout`
    /// elapses. On timeout, the service auto-rejects (writes a
    /// `ToolCallRejected` with `reason = "operator_timeout"`) and returns
    /// [`OperatorDecision::Timeout`].
    async fn await_decision(
        &self,
        call_id: &ToolCallId,
        timeout: Duration,
    ) -> Result<OperatorDecision, RuntimeError>;
}

// ── Match-policy helpers ─────────────────────────────────────────────────────

/// Extract the "path" argument a tool was invoked with. Tools that have
/// a path concept (`read`, `write`, `edit`, `grep`, `glob`) conventionally
/// pass it as a top-level `"path"` field. Returns `None` for tools
/// without that field — the orchestrator is responsible for pinning those
/// to [`ApprovalMatchPolicy::Exact`] at proposal time.
pub(crate) fn extract_path_arg(tool_args: &Value) -> Option<&str> {
    tool_args.get("path").and_then(Value::as_str)
}

/// Canonicalise a path for allow-registry comparison.
///
/// This is a pure lexical canonicalisation — it folds `.` and `..`
/// components, strips trailing separators, and does not touch the
/// filesystem. Symlink resolution is deliberately out of scope here:
/// the allow registry is a boundary check, and a symlink-aware check
/// belongs in the tool-execution fence (mirroring the harness-core
/// `PermissionPolicy.roots` model).
fn canonicalise(p: &str) -> PathBuf {
    let path = Path::new(p);
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Test whether `candidate` is the same path as `root` or lives under it,
/// using path-component boundaries (not raw string prefix). This mirrors
/// the comment on [`ApprovalMatchPolicy::ProjectScopedPath::project_root`]:
/// `/workspaces/cairn2` must not match a root of `/workspaces/cairn`.
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

/// Decide whether `proposal` matches an existing session `rule`.
///
/// * `Exact`: byte-identical `tool_name` + `tool_args`.
/// * `ExactPath`: same `tool_name` AND proposal's path arg canonicalises
///   equal to the rule's path.
/// * `ProjectScopedPath`: same `tool_name` AND proposal's path arg
///   canonicalises under the rule's project root.
pub(crate) fn proposal_matches_rule(proposal: &ToolCallProposal, rule: &AllowRule) -> bool {
    if proposal.tool_name != rule.tool_name {
        return false;
    }
    match &rule.policy {
        ApprovalMatchPolicy::Exact => proposal.tool_args == rule.tool_args,
        ApprovalMatchPolicy::ExactPath { path } => {
            let Some(candidate) = extract_path_arg(&proposal.tool_args) else {
                return false;
            };
            canonicalise(candidate) == canonicalise(path)
        }
        ApprovalMatchPolicy::ProjectScopedPath { project_root } => {
            let Some(candidate) = extract_path_arg(&proposal.tool_args) else {
                return false;
            };
            is_under(&canonicalise(candidate), &canonicalise(project_root))
        }
    }
}

/// Session-scoped allow rule distilled from a `Session`-scope approval.
///
/// Held in memory only; a future PR persists these via projection so
/// that session restart doesn't silently widen the required approvals.
#[derive(Clone, Debug)]
pub struct AllowRule {
    pub tool_name: String,
    /// Arguments captured at approval time. Only used for `Exact` matches.
    pub tool_args: Value,
    pub policy: ApprovalMatchPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(tool: &str, args: Value) -> ToolCallProposal {
        ToolCallProposal {
            call_id: ToolCallId::new("tc_test"),
            session_id: SessionId::new("sess"),
            run_id: RunId::new("run"),
            project: ProjectKey::new("t", "w", "p"),
            tool_name: tool.to_owned(),
            tool_args: args,
            display_summary: None,
            match_policy: ApprovalMatchPolicy::Exact,
        }
    }

    #[test]
    fn exact_rule_matches_byte_identical_args() {
        let p = proposal("read", serde_json::json!({ "path": "/a/b" }));
        let r = AllowRule {
            tool_name: "read".into(),
            tool_args: serde_json::json!({ "path": "/a/b" }),
            policy: ApprovalMatchPolicy::Exact,
        };
        assert!(proposal_matches_rule(&p, &r));
    }

    #[test]
    fn exact_rule_rejects_different_args() {
        let p = proposal("read", serde_json::json!({ "path": "/a/b" }));
        let r = AllowRule {
            tool_name: "read".into(),
            tool_args: serde_json::json!({ "path": "/a/c" }),
            policy: ApprovalMatchPolicy::Exact,
        };
        assert!(!proposal_matches_rule(&p, &r));
    }

    #[test]
    fn exact_rule_rejects_different_tool() {
        let p = proposal("read", serde_json::json!({ "path": "/a/b" }));
        let r = AllowRule {
            tool_name: "write".into(),
            tool_args: serde_json::json!({ "path": "/a/b" }),
            policy: ApprovalMatchPolicy::Exact,
        };
        assert!(!proposal_matches_rule(&p, &r));
    }

    #[test]
    fn exact_path_matches_canonical_equal() {
        let p = proposal("read", serde_json::json!({ "path": "/a/./b/../b" }));
        let r = AllowRule {
            tool_name: "read".into(),
            tool_args: Value::Null,
            policy: ApprovalMatchPolicy::ExactPath {
                path: "/a/b".into(),
            },
        };
        assert!(proposal_matches_rule(&p, &r));
    }

    #[test]
    fn project_scoped_matches_nested_path() {
        let p = proposal("read", serde_json::json!({ "path": "/workspaces/cairn/src/lib.rs" }));
        let r = AllowRule {
            tool_name: "read".into(),
            tool_args: Value::Null,
            policy: ApprovalMatchPolicy::ProjectScopedPath {
                project_root: "/workspaces/cairn".into(),
            },
        };
        assert!(proposal_matches_rule(&p, &r));
    }

    #[test]
    fn project_scoped_rejects_sibling_root() {
        let p = proposal("read", serde_json::json!({ "path": "/workspaces/cairn2/file" }));
        let r = AllowRule {
            tool_name: "read".into(),
            tool_args: Value::Null,
            policy: ApprovalMatchPolicy::ProjectScopedPath {
                project_root: "/workspaces/cairn".into(),
            },
        };
        assert!(
            !proposal_matches_rule(&p, &r),
            "string-prefix bug: /workspaces/cairn2 must not match root /workspaces/cairn"
        );
    }

    #[test]
    fn project_scoped_rejects_path_outside_root() {
        let p = proposal("read", serde_json::json!({ "path": "/etc/passwd" }));
        let r = AllowRule {
            tool_name: "read".into(),
            tool_args: Value::Null,
            policy: ApprovalMatchPolicy::ProjectScopedPath {
                project_root: "/workspaces/cairn".into(),
            },
        };
        assert!(!proposal_matches_rule(&p, &r));
    }

    #[test]
    fn path_rule_rejects_proposal_without_path_arg() {
        let p = proposal("bash", serde_json::json!({ "cmd": "ls" }));
        let r = AllowRule {
            tool_name: "bash".into(),
            tool_args: Value::Null,
            policy: ApprovalMatchPolicy::ExactPath {
                path: "/a".into(),
            },
        };
        assert!(!proposal_matches_rule(&p, &r));
    }
}
