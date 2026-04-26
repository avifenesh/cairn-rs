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
///
/// There is no `AutoRejected` variant yet: a deny-registry is out of scope
/// for PR-3 (the service evaluates a session **allow** registry only).
/// When a deny registry lands in a later PR, a new variant can be added
/// without breaking callers thanks to `#[non_exhaustive]` on the match
/// sites added alongside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Matched an existing session allow-rule — execute with original args.
    AutoApproved,
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

/// Proposal that an operator rejected.
///
/// Surfaced via [`ToolCallApprovalReader::list_rejected_for_run`] so the
/// orchestrator drain can emit a `StepSummary` entry for every rejection
/// the LLM hasn't yet been told about. Without this, the next DECIDE
/// would be blind to the rejection reason and re-propose the same call
/// (the dogfood F46 repro).
#[derive(Clone, Debug)]
pub struct RejectedProposal {
    pub call_id: ToolCallId,
    pub tool_name: String,
    /// Original args the operator rejected. Preserved so the DECIDE
    /// summary can echo *what* was rejected alongside the reason.
    pub tool_args: Value,
    /// Operator-supplied reason. `None` when the rejection omitted one.
    pub reason: Option<String>,
}

/// Resolution state of a proposal as observed from the store projection.
///
/// Mirrors the public projection states so callers rebuilding a runtime
/// cache entry can decide whether to accept the proposal for
/// approve/reject/amend (only `Pending` is accepted; the others already
/// have terminal decisions and must surface as `InvalidTransition`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredProposalState {
    Pending,
    Approved,
    Rejected,
    Timeout,
}

/// Full proposal record re-hydrated from the store projection.
///
/// Used by [`ToolCallApprovalService::approve`] / `reject` / `amend` to
/// recover from in-memory cache misses (e.g. process restart, eviction).
/// Carries everything needed to rebuild a private `ProposalEntry` and
/// proceed with the decision flow.
#[derive(Clone, Debug)]
pub struct StoredProposal {
    pub proposal: ToolCallProposal,
    pub state: StoredProposalState,
    /// Most recent `ToolCallAmended.new_tool_args`, if any.
    pub amended_args: Option<Value>,
    /// `ToolCallApproved.approved_tool_args`, if any.
    pub approved_args: Option<Value>,
    /// `ToolCallRejected.reason`, if any.
    pub rejection_reason: Option<String>,
}

/// Minimal read interface for reconstituting proposals from the store
/// projection (e.g. after process restart or when the in-memory cache
/// has been evicted).
///
/// A blanket impl (below) bridges every store backend's
/// [`cairn_store::projections::ToolCallApprovalReadModel`] into this
/// trait — so any `Arc<InMemoryStore | SqliteAdapter | PgAdapter>` is
/// a valid reader without per-backend glue.
#[async_trait]
pub trait ToolCallApprovalReader: Send + Sync {
    /// Load the materialised approval for a single call.
    async fn get_tool_call_approval(
        &self,
        call_id: &ToolCallId,
    ) -> Result<Option<ApprovedProposal>, RuntimeError>;

    /// Load the full proposal (any state) for re-hydrating the in-memory
    /// cache on approve/reject/amend after restart or eviction.
    ///
    /// Default implementation returns `Ok(None)` so older readers that
    /// only wired `get_tool_call_approval` still compile; production
    /// readers (Postgres/SQLite/InMemory adapters) override this to
    /// return the full projection row. Returning `None` from this hook
    /// causes the service to surface a genuine `NotFound` from the
    /// decision endpoints — matching the pre-fix behaviour.
    async fn get_tool_call_proposal(
        &self,
        _call_id: &ToolCallId,
    ) -> Result<Option<StoredProposal>, RuntimeError> {
        Ok(None)
    }

    /// F25 drain: list every `Approved`-state tool-call approval for
    /// `run_id`, oldest-first. The orchestrator uses this at the top of
    /// `run_inner` to decide whether any operator-approved proposals
    /// still need to be dispatched before the next DECIDE turn.
    ///
    /// Callers are responsible for filtering out proposals whose
    /// `ToolCallId` already has a matching `CachedToolResult` (i.e. the
    /// tool already ran in a previous loop iteration or in a previous
    /// process before a restart). The projection alone cannot know that
    /// — only the runtime-side `ToolCallResultCache` tracks
    /// per-run dispatch completions keyed by `ToolCallId`.
    ///
    /// Default impl returns `Ok(vec![])` so older readers keep compiling;
    /// the runtime treats an empty list as "nothing to drain" which is
    /// the legacy behaviour.
    async fn list_approved_for_run(
        &self,
        _run_id: &cairn_domain::RunId,
    ) -> Result<Vec<ApprovedProposal>, RuntimeError> {
        Ok(Vec::new())
    }

    /// F46 drain: list every `Rejected`-state tool-call approval for
    /// `run_id`, oldest-first. The orchestrator surfaces these as
    /// `StepSummary` entries so the next DECIDE sees the rejection
    /// reason verbatim and doesn't re-propose the same call. Returns an
    /// empty vec by default so readers that have not opted in stay
    /// feature-equivalent to their pre-F46 behaviour.
    async fn list_rejected_for_run(
        &self,
        _run_id: &cairn_domain::RunId,
    ) -> Result<Vec<RejectedProposal>, RuntimeError> {
        Ok(Vec::new())
    }
}

/// Bridge every store that implements
/// [`cairn_store::projections::ToolCallApprovalReadModel`] into the
/// runtime-level [`ToolCallApprovalReader`]. Enforces the effective-args
/// precedence (approved → amended → original) here so orchestrator
/// callers receive the payload the execute phase should actually run.
#[async_trait]
impl<T> ToolCallApprovalReader for T
where
    T: cairn_store::projections::ToolCallApprovalReadModel + Send + Sync + ?Sized,
{
    async fn get_tool_call_approval(
        &self,
        call_id: &ToolCallId,
    ) -> Result<Option<ApprovedProposal>, RuntimeError> {
        let record = cairn_store::projections::ToolCallApprovalReadModel::get(self, call_id)
            .await
            .map_err(RuntimeError::Store)?;
        let Some(r) = record else {
            return Ok(None);
        };
        // Critical: only surface Approved records. Pending/Rejected/Timeout
        // rows must not leak into the runtime's "approved" fast path —
        // otherwise `retrieve_approved_proposal` on a cache miss could
        // execute a tool call that was never approved (or was rejected).
        if r.state != cairn_store::projections::ToolCallApprovalState::Approved {
            return Ok(None);
        }
        Ok(Some(ApprovedProposal {
            call_id: r.call_id.clone(),
            tool_name: r.tool_name.clone(),
            tool_args: r
                .approved_tool_args
                .clone()
                .or_else(|| r.amended_tool_args.clone())
                .unwrap_or_else(|| r.original_tool_args.clone()),
        }))
    }

    async fn get_tool_call_proposal(
        &self,
        call_id: &ToolCallId,
    ) -> Result<Option<StoredProposal>, RuntimeError> {
        let record = cairn_store::projections::ToolCallApprovalReadModel::get(self, call_id)
            .await
            .map_err(RuntimeError::Store)?;
        Ok(record.map(store_record_to_stored_proposal))
    }

    async fn list_approved_for_run(
        &self,
        run_id: &cairn_domain::RunId,
    ) -> Result<Vec<ApprovedProposal>, RuntimeError> {
        let rows = cairn_store::projections::ToolCallApprovalReadModel::list_for_run(self, run_id)
            .await
            .map_err(RuntimeError::Store)?;
        // Filter to Approved-state rows and resolve the effective args
        // per the domain precedence invariant (approved > amended > original).
        // Pending / Rejected / Timeout rows must NOT leak into the drain
        // path — doing so would execute a tool the operator did not
        // sanction. `list_for_run` is oldest-first; preserve that order so
        // drain replays approvals in the sequence they were minted.
        use cairn_store::projections::ToolCallApprovalState as S;
        Ok(rows
            .into_iter()
            .filter(|r| r.state == S::Approved)
            .map(|r| {
                let tool_args = r
                    .approved_tool_args
                    .clone()
                    .or_else(|| r.amended_tool_args.clone())
                    .unwrap_or_else(|| r.original_tool_args.clone());
                ApprovedProposal {
                    call_id: r.call_id,
                    tool_name: r.tool_name,
                    tool_args,
                }
            })
            .collect())
    }

    async fn list_rejected_for_run(
        &self,
        run_id: &cairn_domain::RunId,
    ) -> Result<Vec<RejectedProposal>, RuntimeError> {
        let rows = cairn_store::projections::ToolCallApprovalReadModel::list_for_run(self, run_id)
            .await
            .map_err(RuntimeError::Store)?;
        use cairn_store::projections::ToolCallApprovalState as S;
        Ok(rows
            .into_iter()
            .filter(|r| r.state == S::Rejected)
            .map(|r| {
                // Echo the args the operator saw — prefer amended over
                // original so the DECIDE summary captures any tweaks the
                // operator reviewed before rejecting.
                let tool_args = r
                    .amended_tool_args
                    .clone()
                    .unwrap_or_else(|| r.original_tool_args.clone());
                RejectedProposal {
                    call_id: r.call_id,
                    tool_name: r.tool_name,
                    tool_args,
                    reason: r.reason,
                }
            })
            .collect())
    }
}

/// Map a projection row into the runtime-facing [`StoredProposal`]. The
/// mapping intentionally preserves the three argument slots separately
/// (original / amended / approved) so the runtime can re-apply the
/// domain precedence invariant without re-reading the projection.
pub(crate) fn store_record_to_stored_proposal(
    r: cairn_store::projections::ToolCallApprovalRecord,
) -> StoredProposal {
    let state = match r.state {
        cairn_store::projections::ToolCallApprovalState::Pending => StoredProposalState::Pending,
        cairn_store::projections::ToolCallApprovalState::Approved => StoredProposalState::Approved,
        cairn_store::projections::ToolCallApprovalState::Rejected => StoredProposalState::Rejected,
        cairn_store::projections::ToolCallApprovalState::Timeout => StoredProposalState::Timeout,
    };
    StoredProposal {
        proposal: ToolCallProposal {
            call_id: r.call_id,
            session_id: r.session_id,
            run_id: r.run_id,
            project: r.project,
            tool_name: r.tool_name,
            tool_args: r.original_tool_args,
            display_summary: r.display_summary,
            match_policy: r.match_policy,
        },
        state,
        amended_args: r.amended_tool_args,
        approved_args: r.approved_tool_args,
        rejection_reason: r.reason,
    }
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
///
/// A `..` component is only folded when the accumulated path has a
/// non-root tail to pop. `/../a` canonicalises to `/a`, not `a`; a
/// naive `out.pop()` would eat the leading `RootDir` and produce a
/// relative path that silently changes `ExactPath` equality and
/// `ProjectScopedPath` containment.
fn canonicalise(p: &str) -> PathBuf {
    let path = Path::new(p);
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                // Only pop an already-pushed non-anchor component. If
                // the tail is `RootDir`/`Prefix` (or empty), ignore.
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
        let p = proposal(
            "read",
            serde_json::json!({ "path": "/workspaces/cairn/src/lib.rs" }),
        );
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
        let p = proposal(
            "read",
            serde_json::json!({ "path": "/workspaces/cairn2/file" }),
        );
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
    fn canonicalise_keeps_root_when_parent_dir_would_escape() {
        // `/../a` must canonicalise to `/a`, not `a`. If `..` could pop
        // the `RootDir`, `ExactPath` equality and `ProjectScopedPath`
        // containment would silently change meaning.
        let p = proposal("read", serde_json::json!({ "path": "/../a" }));
        let r = AllowRule {
            tool_name: "read".into(),
            tool_args: Value::Null,
            policy: ApprovalMatchPolicy::ExactPath { path: "/a".into() },
        };
        assert!(proposal_matches_rule(&p, &r));
    }

    #[test]
    fn path_rule_rejects_proposal_without_path_arg() {
        let p = proposal("bash", serde_json::json!({ "cmd": "ls" }));
        let r = AllowRule {
            tool_name: "bash".into(),
            tool_args: Value::Null,
            policy: ApprovalMatchPolicy::ExactPath { path: "/a".into() },
        };
        assert!(!proposal_matches_rule(&p, &r));
    }
}
