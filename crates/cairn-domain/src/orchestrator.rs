//! Orchestrator types — ActionProposal from the DECIDE phase.
//!
//! The orchestrator's DECIDE phase calls the LLM with a `ContextBundle` and
//! receives an `ActionProposal` back.  This module defines the proposal shape
//! and the exhaustive set of action types the orchestrator can execute.
//!
//! Also houses [`CompletionVerification`] (F47): an extractor-produced sidecar
//! that scans tool_result frames at Done and surfaces warning/error lines and
//! per-command exit codes so operators have an independent signal alongside
//! the LLM's free-text `complete_run` summary.

use serde::{Deserialize, Serialize};

// ── ActionType ────────────────────────────────────────────────────────────────

/// The class of action the orchestrator wants to take next.
///
/// Each variant maps to a distinct runtime capability that the execution layer
/// knows how to dispatch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    /// Spawn a subordinate agent to handle a sub-task.
    SpawnSubagent,
    /// Invoke a registered tool (function call, plugin, external API).
    InvokeTool,
    /// Persist new knowledge into the memory store.
    CreateMemory,
    /// Emit a notification to an operator or downstream channel.
    SendNotification,
    /// Mark the current run as successfully completed.
    CompleteRun,
    /// Escalate to a human operator when the agent is stuck or uncertain.
    EscalateToOperator,
}

// ── ActionProposal ────────────────────────────────────────────────────────────

/// What the LLM returns from the DECIDE phase.
///
/// The orchestrator deserialises the LLM's structured output into this type.
/// The execution layer reads `action_type` and dispatches accordingly.
///
/// `confidence` is the raw value from the LLM — callers should apply a
/// `CalibrationAdjustment` before acting on it (see cairn-runtime's
/// `ConfidenceCalibrator`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionProposal {
    /// Which action the LLM decided to take.
    pub action_type: ActionType,
    /// Human-readable explanation of the decision (for audit and UI display).
    pub description: String,
    /// Raw predicted confidence in this action [0.0, 1.0].
    pub confidence: f64,
    /// Tool name when `action_type == InvokeTool` or `SpawnSubagent`.
    /// `None` for actions that don't target a named tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// JSON arguments to pass to the tool or sub-agent.
    /// `None` when not applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_args: Option<serde_json::Value>,
    /// Whether this proposal must be approved by an operator before execution.
    ///
    /// The orchestrator sets this based on the configured approval policy.
    /// `true` gates execution on an `ApprovalResolved` event.
    pub requires_approval: bool,
}

impl ActionProposal {
    /// Construct a minimal complete-run proposal.
    pub fn complete_run(description: impl Into<String>, confidence: f64) -> Self {
        Self {
            action_type: ActionType::CompleteRun,
            description: description.into(),
            confidence,
            tool_name: None,
            tool_args: None,
            requires_approval: false,
        }
    }

    /// Construct a tool invocation proposal.
    pub fn invoke_tool(
        tool_name: impl Into<String>,
        tool_args: serde_json::Value,
        description: impl Into<String>,
        confidence: f64,
        requires_approval: bool,
    ) -> Self {
        Self {
            action_type: ActionType::InvokeTool,
            description: description.into(),
            confidence,
            tool_name: Some(tool_name.into()),
            tool_args: Some(tool_args),
            requires_approval,
        }
    }

    /// Construct an escalate-to-operator proposal.
    pub fn escalate(description: impl Into<String>, confidence: f64) -> Self {
        Self {
            action_type: ActionType::EscalateToOperator,
            description: description.into(),
            confidence,
            tool_name: None,
            tool_args: None,
            requires_approval: true,
        }
    }
}

// Manual Eq for f64 field (confidence). Used in tests for equality checks.
// Bit-exact equality is acceptable for test assertions.
impl PartialEq for ActionProposal {
    fn eq(&self, other: &Self) -> bool {
        self.action_type == other.action_type
            && self.description == other.description
            && self.confidence.to_bits() == other.confidence.to_bits()
            && self.tool_name == other.tool_name
            && self.requires_approval == other.requires_approval
    }
}

impl Eq for ActionProposal {}

// ── CompletionVerification (F47 PR1) ──────────────────────────────────────────

/// Independent evidence distilled from tool_result frames at run Done.
///
/// F47 motivation: during dogfood M1 (2026-04-26) an LLM produced a Rust crate
/// that emitted `warning: unused imports: …` in a stored bash tool_result, then
/// claimed `cargo check must pass with no warnings` in its `complete_run`
/// summary. Operators had no independent signal that the summary lied. This
/// type is extracted by a pure scanner over tool_result frames and attached to
/// `LoopTermination::Completed` so SSE watchers see the evidence alongside the
/// free-text summary.
///
/// Intentionally non-authoritative: the extractor reports what the tool
/// outputs say, not whether the run "succeeded." The orchestrator's loop
/// signal is still the source of truth for run state.
///
/// `extractor_version` identifies the scanner logic that produced the bucket.
/// Bump when the regex-set or truncation policy changes so downstream consumers
/// can distinguish shapes across cairn versions.
///
/// PR1 (this change) makes this sidecar visible on the SSE `finished` event.
/// PR2 adds persistence + REST. PR3 adds UI rendering.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionVerification {
    /// Tool-output lines matched by the warning signal — lines starting
    /// with the case-insensitive ASCII prefix `warning:` (e.g. `warning:
    /// unused import`). Broader forms like `WARN:` / `Warn` are NOT
    /// currently matched; the scanner is tuned for `rustc`/`cargo`/
    /// `clippy` plus the generic `warning:` convention, which covers the
    /// F47 M1 regression. Full matched line, truncated to 500 chars,
    /// capped at 50 entries.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Tool-output lines matched by the error signal (e.g. Rust
    /// `error[E0308]: …`, `error: …`). Same truncation / cap rules.
    #[serde(default)]
    pub errors: Vec<String>,
    /// Per-bash-class-tool outcome: the command that ran and its exit
    /// code if the tool_result carried one. Only bash-class tools
    /// (`bash`, `shell_exec`, `run_bash`) produce entries here; non-bash
    /// tools still contribute to `warnings`/`errors` via their text
    /// output but are not listed in this vector. `exit_code: None` means
    /// the frame did not structurally expose an exit code; do not infer
    /// success from its absence.
    #[serde(default)]
    pub commands: Vec<CommandOutcome>,
    /// How many tool_result frames the extractor scanned to produce this
    /// summary. `0` means "Done reached with no recorded tool_results" —
    /// distinct from "scanned frames but found nothing."
    pub tool_results_scanned: usize,
    /// Version of the extractor logic. Bump when the bucket shape or
    /// scanning policy changes. v1 = F47 PR1.
    pub extractor_version: u32,
}

/// One bash-class tool invocation's command and exit code, as observed in
/// a tool_result frame. Only bash-class tools produce `CommandOutcome`
/// entries — non-bash tools contribute to `warnings`/`errors` via their
/// text output but never appear here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutcome {
    /// Tool name from the proposal (e.g. `"bash"`, `"shell_exec"`).
    pub tool_name: String,
    /// The `command` arg from the proposal, truncated to 500 chars. Empty
    /// when the proposal did not carry a `command` field.
    pub cmd: String,
    /// Exit code surfaced by the tool_result, if the structured payload
    /// carried one. `None` = not present in the frame; callers MUST NOT
    /// fabricate (e.g. do not default to `0` on success).
    pub exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_type_serialises_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ActionType::SpawnSubagent).unwrap(),
            r#""spawn_subagent""#
        );
        assert_eq!(
            serde_json::to_string(&ActionType::EscalateToOperator).unwrap(),
            r#""escalate_to_operator""#
        );
    }

    #[test]
    fn action_proposal_round_trips_json() {
        let proposal = ActionProposal::invoke_tool(
            "web_search",
            serde_json::json!({ "query": "Rust async patterns" }),
            "search the web for context",
            0.82,
            false,
        );
        let json = serde_json::to_string(&proposal).unwrap();
        let decoded: ActionProposal = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.action_type, ActionType::InvokeTool);
        assert_eq!(decoded.tool_name.as_deref(), Some("web_search"));
        assert!(!decoded.requires_approval);
    }

    #[test]
    fn complete_run_builder_sets_no_tool() {
        let p = ActionProposal::complete_run("all tasks done", 0.95);
        assert_eq!(p.action_type, ActionType::CompleteRun);
        assert!(p.tool_name.is_none());
        assert!(p.tool_args.is_none());
        assert!(!p.requires_approval);
    }

    #[test]
    fn escalate_builder_requires_approval() {
        let p = ActionProposal::escalate("stuck on ambiguous requirement", 0.2);
        assert_eq!(p.action_type, ActionType::EscalateToOperator);
        assert!(p.requires_approval);
    }
}
