//! Tool-call approval primitives shared by runtime, API, and operator surfaces.
//!
//! This module contains the **type foundation** for tool-call approval
//! workflows (distinct from the more general [`crate::policy::ApprovalDecision`]
//! / [`crate::events::ApprovalRequested`] pair that services plan review,
//! prompt-release, RFC 022 triggers, and so on).
//!
//! Two enums live here:
//!
//! * [`ApprovalMatchPolicy`] — how to match *future* tool calls against an
//!   operator decision. `Exact` repeats the decision only for byte-identical
//!   arguments; `ProjectScopedPath` widens the match to any call whose path
//!   argument is under a given project root; `ExactPath` widens to any call
//!   whose path argument equals a specific absolute path.
//!
//! * [`ApprovalScope`] — whether an operator decision is `Once` (this call
//!   only) or `Session { match_policy }` (all subsequent matching calls in
//!   the same session).
//!
//! These types are additive — they are introduced in the "tool-call approval"
//! event foundation wave (PR BP-1) and are **not** consumed by any service
//! yet. Subsequent PRs in the wave wire them into `ToolCallProposed`,
//! `ToolCallApproved`, projection state, and the execute phase.
//!
//! Design notes:
//!
//! * Both types use `#[serde(tag = "kind", rename_all = "snake_case")]` so
//!   that the JSON wire shape is a discriminated union — operators and
//!   plugin authors read these payloads directly via SSE / audit log.
//! * Paths are carried as owned `String`s (not `PathBuf`) to keep the type
//!   serde-friendly and portable across OS boundaries. Canonicalisation is
//!   the concern of whichever matcher consumes the policy, not the policy
//!   itself.

use serde::{Deserialize, Serialize};

/// How a tool-call approval decision should match *future* calls.
///
/// This enum is referenced by [`crate::events::ToolCallProposed`]
/// (what the operator is being asked to match on) and
/// [`ApprovalScope::Session`] (how a "approve for session" decision widens).
///
/// The variants are ordered from narrowest (`Exact`) to widest
/// (`ProjectScopedPath`). The matcher is expected to compare arguments
/// *after* canonicalisation (symlink resolution, trailing-slash handling,
/// etc.) — that canonicalisation is deliberately not encoded in the type
/// so that the same policy can be reused in contexts where only a subset
/// of canonicalisation is available (e.g. replay).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalMatchPolicy {
    /// Match only tool calls whose canonical tool-arguments payload is
    /// byte-identical to the originally-approved call.
    Exact,
    /// Match any tool call whose path argument is inside the given
    /// project root (inclusive). Useful for "approve reads inside this
    /// workspace" flows.
    ProjectScopedPath {
        /// Canonical absolute path of the project root. The matcher
        /// compares `candidate_path.starts_with(project_root)` after
        /// canonicalising both sides.
        project_root: String,
    },
    /// Match any tool call whose path argument is exactly the given
    /// absolute path (post-canonicalisation). Useful for "approve
    /// edits to this one file for the rest of the session" flows.
    ExactPath {
        /// Canonical absolute path of the approved target file.
        path: String,
    },
}

/// Scope of an operator approval decision over a proposed tool call.
///
/// `Once` is the default safe choice: the decision applies only to the
/// specific `ToolCallId` the operator inspected. `Session` widens the
/// decision to every subsequent tool call in the same session whose
/// arguments satisfy the embedded [`ApprovalMatchPolicy`].
///
/// There is intentionally no `Global` or `Project` scope in this PR — those
/// broaden blast radius beyond the session and need additional policy /
/// audit machinery that will be introduced in later PRs of the wave, if at
/// all. Keeping the initial enum tight makes adding variants later an
/// additive (non-breaking) change on the deserialise side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalScope {
    /// Decision applies to this one tool call only.
    Once,
    /// Decision applies to every tool call in the same session whose
    /// canonicalised arguments satisfy `match_policy`.
    Session {
        /// How subsequent tool calls are compared against the approved one.
        match_policy: ApprovalMatchPolicy,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_match_policy_roundtrips_through_json() {
        let cases = vec![
            ApprovalMatchPolicy::Exact,
            ApprovalMatchPolicy::ProjectScopedPath {
                project_root: "/workspaces/cairn".to_owned(),
            },
            ApprovalMatchPolicy::ExactPath {
                path: "/workspaces/cairn/README.md".to_owned(),
            },
        ];
        for policy in cases {
            let json = serde_json::to_string(&policy).expect("serialize");
            let back: ApprovalMatchPolicy = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(policy, back);
        }
    }

    #[test]
    fn approval_scope_roundtrips_through_json() {
        let cases = vec![
            ApprovalScope::Once,
            ApprovalScope::Session {
                match_policy: ApprovalMatchPolicy::Exact,
            },
            ApprovalScope::Session {
                match_policy: ApprovalMatchPolicy::ProjectScopedPath {
                    project_root: "/tmp/proj".to_owned(),
                },
            },
        ];
        for scope in cases {
            let json = serde_json::to_string(&scope).expect("serialize");
            let back: ApprovalScope = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(scope, back);
        }
    }

    #[test]
    fn approval_match_policy_uses_snake_case_kind_tag() {
        let json = serde_json::to_string(&ApprovalMatchPolicy::ProjectScopedPath {
            project_root: "/p".to_owned(),
        })
        .expect("serialize");
        assert!(
            json.contains("\"kind\":\"project_scoped_path\""),
            "expected snake_case kind tag, got {json}"
        );
    }

    #[test]
    fn approval_scope_session_serializes_nested_policy() {
        let scope = ApprovalScope::Session {
            match_policy: ApprovalMatchPolicy::ExactPath {
                path: "/a/b.rs".to_owned(),
            },
        };
        let json = serde_json::to_string(&scope).expect("serialize");
        assert!(json.contains("\"kind\":\"session\""));
        assert!(json.contains("\"match_policy\""));
        assert!(json.contains("\"kind\":\"exact_path\""));
        assert!(json.contains("\"path\":\"/a/b.rs\""));
    }
}
