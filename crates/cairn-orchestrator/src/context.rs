//! Core types shared across all orchestrator phases.
//!
//! These types represent the data flowing between GatherPhase → DecidePhase
//! → ExecutePhase, plus the loop control signals and configuration.

use std::path::PathBuf;

use cairn_domain::{
    decisions::RunMode, ActionProposal, ApprovalId, DefaultSetting, ProjectKey, RunId, SessionId,
    TaskId, ToolInvocationId,
};
use cairn_graph::GraphNode;
use cairn_memory::retrieval::RetrievalResult;
use cairn_store::projections::CheckpointRecord;
use cairn_store::StoredEvent;
use serde::{Deserialize, Serialize};

// ── OrchestrationContext ──────────────────────────────────────────────────────

/// Immutable context threaded through every phase of a single iteration.
///
/// Built once per run (or rebuilt from a checkpoint on resume) and passed
/// by reference to all three phases.
#[derive(Clone, Debug)]
pub struct OrchestrationContext {
    /// The project this execution belongs to.
    pub project: ProjectKey,
    pub session_id: SessionId,
    pub run_id: RunId,
    /// The task that holds the current execution lease, if any.
    pub task_id: Option<TaskId>,
    /// Which iteration of the loop we are on (0-based).
    pub iteration: u32,
    /// The original user goal / input message that started this run.
    pub goal: String,
    /// Agent type label used for routing, confidence calibration, and logging.
    pub agent_type: String,
    /// Wall-clock millisecond timestamp when this run began (for timeout checks).
    pub run_started_at_ms: u64,
    /// Working directory used for filesystem and process-oriented tools.
    pub working_dir: PathBuf,
    /// Execution mode for this run (RFC 018).
    ///
    /// - `Direct` — all tools visible, agent acts freely.
    /// - `Plan` — only Observational + Internal tools visible; run terminates
    ///   with `plan_proposed` when agent emits `<proposed_plan>`.
    /// - `Execute { plan_run_id }` — seeded with an approved plan artifact.
    pub run_mode: RunMode,
    /// Deferred tool names discovered via `tool_search` in previous iterations.
    ///
    /// The loop runner populates this after execute when it detects a
    /// `tool_search` result with matches.  `LlmDecidePhase` reads these names
    /// and injects their descriptors into the *next* iteration's system prompt,
    /// making discovered tools visible to the LLM without permanently promoting
    /// them to Registered tier.
    pub discovered_tool_names: Vec<String>,
    /// T5-H1: step summaries from prior iterations of this run, threaded
    /// from the loop runner into each iteration's gather phase so the
    /// LLM sees its own recent history. The loop mutates this in place
    /// after every iteration; gather copies it into `GatherOutput.step_history`
    /// (optionally merged with checkpointed entries on resume).
    pub step_history: Vec<StepSummary>,
    /// RFC 020 Track 3: true when this iteration is replaying after a crash.
    ///
    /// Set by the loop runner on entry to the execute phase if the run was
    /// resumed from a checkpoint. The execute phase consults this (together
    /// with the tool's `RetrySafety` classification) to decide whether to
    /// silently re-dispatch, serve from cache, or pause for operator approval.
    ///
    /// Defaults to `false` for fresh (non-recovered) runs. Set only for the
    /// first post-recovery iteration; subsequent iterations use `false`.
    pub is_recovery: bool,

    /// Maximum wall-clock the execute phase will wait on an operator
    /// approval before auto-rejecting the proposal. Threaded from
    /// `OrchestrateRequest.approval_timeout_ms` (HTTP) or defaulted to
    /// 24h when the caller didn't provide one.
    ///
    /// `None` means "never set" — downstream defaults to 24h. We keep the
    /// field `Option<Duration>` so `Default` / `Clone`-constructed contexts
    /// continue to work without having to pick a sentinel value here.
    pub approval_timeout: Option<std::time::Duration>,
}

// ── GatherOutput ─────────────────────────────────────────────────────────────

/// Context snapshot produced by the GatherPhase.
///
/// Everything the DecidePhase needs to build a decision prompt is
/// contained in this struct.
#[derive(Clone, Debug, Default)]
pub struct GatherOutput {
    /// Relevant memory chunks from semantic + lexical retrieval.
    /// Source: `cairn_memory::RetrievalService::query`
    pub memory_chunks: Vec<RetrievalResult>,

    /// Recent events for the current run (tool calls, approvals, checkpoints).
    /// Source: `cairn_store::EventLog::read_by_entity(EntityRef::Run(…))`
    pub recent_events: Vec<StoredEvent>,

    /// Graph neighbourhood nodes linked to this run/session (depth ≤ 2).
    /// Source: `cairn_graph::GraphQueryService`
    pub graph_nodes: Vec<GraphNode>,

    /// Operator settings that apply to this project/tenant.
    /// Source: `DefaultsReadModel::list_by_scope(Scope::Project, …)`
    pub operator_settings: Vec<DefaultSetting>,

    /// Most recent checkpoint for this run (for resume awareness).
    /// Source: `CheckpointService::latest_for_run`
    pub checkpoint: Option<CheckpointRecord>,

    /// Compressed summaries of prior steps in this run.
    pub step_history: Vec<StepSummary>,
}

/// A compressed record of what happened in a prior orchestration step.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepSummary {
    pub iteration: u32,
    /// High-level kind: "tool_call", "subagent", "continue", "approval_wait", etc.
    pub action_kind: String,
    /// Human-readable or LLM-generated summary of the step.
    pub summary: String,
    /// Whether the step succeeded, failed, or is still pending.
    pub succeeded: bool,
}

// ── DecideOutput ─────────────────────────────────────────────────────────────

/// The proposed action set produced by the DecidePhase after calling the LLM.
#[derive(Clone, Debug)]
pub struct DecideOutput {
    /// Raw LLM response text (retained for audit and replay).
    pub raw_response: String,
    /// Structured actions the LLM proposed (parsed from `raw_response`).
    /// Re-uses `ActionProposal` from `cairn_domain::orchestrator`.
    pub proposals: Vec<ActionProposal>,
    /// Calibrated confidence after applying `ConfidenceCalibrator` adjustments.
    pub calibrated_confidence: f64,
    /// Whether any proposal requires operator approval before execution.
    pub requires_approval: bool,
    /// LLM model ID used for this decision.
    pub model_id: String,
    /// Round-trip latency of the LLM call in milliseconds.
    pub latency_ms: u64,
    /// Input (prompt) token count from the provider response.
    pub input_tokens: Option<u32>,
    /// Output (completion) token count from the provider response.
    pub output_tokens: Option<u32>,
}

// ── ExecuteOutcome ────────────────────────────────────────────────────────────

/// What actually happened after running the proposals from `DecideOutput`.
#[derive(Clone, Debug)]
pub struct ExecuteOutcome {
    /// Per-proposal results, in the same order as `decide_output.proposals`.
    pub results: Vec<ActionResult>,
    /// Loop control signal derived from the execution results.
    pub loop_signal: LoopSignal,
}

/// The result of executing one `ActionProposal`.
#[derive(Clone, Debug)]
pub struct ActionResult {
    /// The proposal that was (attempted to be) executed.
    pub proposal: ActionProposal,
    /// What happened when we tried to execute it.
    pub status: ActionStatus,
    /// For tool calls: the raw tool output (observation for next gather).
    pub tool_output: Option<serde_json::Value>,
    /// The `ToolInvocationId` recorded in the event log (for replay linkage).
    pub invocation_id: Option<ToolInvocationId>,
    /// Wall-clock duration of the dispatch, in milliseconds. Stamped by the
    /// `ExecutePhase` caller after `dispatch_one` returns; inner construction
    /// sites initialise to 0 and the outer wrapper overwrites with the real
    /// elapsed value.
    ///
    /// **0 means "unknown / below-timer-resolution" or "result was synthesised
    /// by a test stub that did not actually dispatch,"** NOT "the tool ran in
    /// literally zero time." Downstream consumers (FF attempt-stream replay,
    /// cost reconciliation, latency percentiles) MUST treat 0 as no-signal
    /// rather than zero-duration; averaging or percentile computations that
    /// trust 0-as-measured will under-report latency. The same convention
    /// applies to the `tool_result` frame the loop emits to FF's
    /// attempt_stream — see `loop_runner.rs` where this field feeds
    /// `log_tool_result`.
    pub duration_ms: u64,
}

/// Status of a single executed action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionStatus {
    /// The action completed without error.
    Succeeded,
    /// The action produced an error.
    Failed { reason: String },
    /// The action is blocked on an operator approval decision.
    /// The run has been transitioned to `waiting_approval`.
    AwaitingApproval { approval_id: ApprovalId },
    /// A subagent was spawned.  `child_task_id` is the schedulable unit.
    SubagentSpawned { child_task_id: TaskId },
}

// ── LoopSignal ────────────────────────────────────────────────────────────────

/// Tells `OrchestratorLoop` what to do after an `ExecutePhase` completes.
///
/// Mirrors `cairn_agent::react::LoopSignal` but defined here to avoid
/// a hard dependency on cairn-agent internals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoopSignal {
    /// Continue to the next iteration.
    Continue,
    /// Agent declared its work done.
    Done,
    /// Agent (or execute phase) hit an unrecoverable error.
    Failed { reason: String },
    /// Run is now blocked on an approval; loop suspends.
    WaitApproval { approval_id: ApprovalId },
    /// Run is blocked waiting for a spawned subagent to finish.
    WaitSubagent { child_task_id: TaskId },
    /// Plan-mode run: agent emitted a `<proposed_plan>` block (RFC 018).
    PlanProposed { plan_markdown: String },
}

impl LoopSignal {
    /// Canonical snake_case tag for telemetry/SSE, matching the rest of
    /// the cairn-domain `#[serde(rename_all = "snake_case")]` convention.
    pub fn kind(&self) -> &'static str {
        match self {
            LoopSignal::Continue => "continue",
            LoopSignal::Done => "done",
            LoopSignal::Failed { .. } => "failed",
            LoopSignal::WaitApproval { .. } => "wait_approval",
            LoopSignal::WaitSubagent { .. } => "wait_subagent",
            LoopSignal::PlanProposed { .. } => "plan_proposed",
        }
    }
}

// ── LoopTermination ──────────────────────────────────────────────────────────

/// Why `OrchestratorLoop::run` returned.
///
/// The HTTP handler (or task worker) inspects this to decide what to do
/// with the run and task lease.
#[derive(Clone, Debug)]
pub enum LoopTermination {
    /// Agent declared itself done; run has been completed.
    Completed { summary: String },
    /// Agent or runtime hit an unrecoverable error; run has been failed.
    Failed { reason: String },
    /// Iteration cap reached; run has been failed with `MaxIterations`.
    MaxIterationsReached,
    /// Wall-clock timeout; run has been failed with `Timeout`.
    TimedOut,
    /// Run is waiting for operator approval; loop suspended.
    /// Resume by re-entering the loop after `ApprovalResolved`.
    WaitingApproval { approval_id: ApprovalId },
    /// Run is waiting for a child subagent.  Loop suspended.
    /// Resume when the child task completes (dependency sweep).
    WaitingSubagent { child_task_id: TaskId },
    /// Plan-mode run completed with a plan artifact (RFC 018).
    /// The run is Completed with outcome `plan_proposed`.
    PlanProposed { plan_markdown: String },
}

// ── LoopConfig ───────────────────────────────────────────────────────────────

/// Tuning parameters for `OrchestratorLoop`.
#[derive(Clone, Debug)]
pub struct LoopConfig {
    /// Maximum iterations before the loop fails with `MaxIterations`.
    pub max_iterations: u32,
    /// Wall-clock timeout for the entire run in milliseconds.
    pub timeout_ms: u64,
    /// Save a checkpoint after every N tool calls (0 = every step).
    pub checkpoint_every_n_tool_calls: u32,
    /// Context compaction settings (RFC 018).
    pub compaction: CompactionConfig,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 20,
            timeout_ms: 5 * 60 * 1_000, // 5 minutes
            checkpoint_every_n_tool_calls: 1,
            compaction: CompactionConfig::default(),
        }
    }
}

// ── CompactionConfig ────────────────────────────────────────────────────────

/// Configuration for inline context compaction (RFC 018 Enhancement 3).
///
/// When the step history exceeds `threshold_pct` of the estimated context
/// budget, the loop runner triggers a summarization turn to compress
/// older history into a compact summary, preserving the most recent
/// `keep_last` steps verbatim.
#[derive(Clone, Debug)]
pub struct CompactionConfig {
    /// Whether compaction is enabled.
    pub enabled: bool,
    /// Trigger compaction when step_history tokens exceed this percentage
    /// of the model's context window (0–100).
    pub threshold_pct: u32,
    /// Minimum number of steps before compaction can trigger.
    pub min_steps: usize,
    /// Always keep the most recent N steps verbatim (not compacted).
    pub keep_last: usize,
    /// Maximum tokens for the compacted summary.
    pub summary_token_budget: usize,
    /// Minimum number of iterations between compaction passes.
    ///
    /// This prevents runaway compaction on every turn when the threshold is
    /// set aggressively low.
    pub cooldown_iterations: u32,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_pct: 70,
            min_steps: 10,
            keep_last: 4,
            summary_token_budget: 2000,
            cooldown_iterations: 5,
        }
    }
}
