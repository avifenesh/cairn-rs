//! Cairn-side abstraction over FF's control-plane FCALLs.
//!
//! # Why a separate trait (vs folding into [`Engine`])
//!
//! [`Engine`](super::Engine) owns read-side snapshots and tag writes —
//! pure HGETALL / HGET / HSET shaped ops over FF-owned state. The
//! operations defined here are FCALL-shaped: they drive FF's Lua
//! library (`ff_create_budget`, `ff_report_usage_and_check`,
//! `ff_check_admission_and_record`, `ff_rotate_waitpoint_hmac_secret`)
//! and surface typed outcome enums that require per-FCALL envelope
//! parsing.
//!
//! Mixing the two would give a single trait ~20 methods that split
//! cleanly along read-vs-FCALL lines; splitting here keeps each trait
//! focused and lets callers take the narrowest dep they need (e.g. a
//! view-layer worker only needs `Engine`, not `ControlPlaneBackend`).
//!
//! # Phase D scope
//!
//! - **PR 1**: budget (5), quota (2), rotation (1).
//! - **PR 2a**: run lifecycle (7) + session create/archive (2) +
//!   claim (1). All FCALL-shaped; fold onto this trait rather than a new
//!   `ExecutionLifecycleBackend` — one trait keeps services from
//!   juggling two handles, and the read/tag split (Engine) vs
//!   FCALL split (ControlPlaneBackend) is already the load-bearing
//!   axis.
//! - **PR 2b** (follow-up): task lifecycle (10+ methods including
//!   `declare_dependency` retry loop + `check_dependencies` envelope
//!   walk). Deferred because that service has behaviour — multi-FCALL
//!   retry, graph-revision conflict recovery — that deserves its own
//!   scope audit.
//!
//! # Error model
//!
//! Every method returns [`FabricError`] for transport / serialisation /
//! malformed-envelope failures. Typed FF outcomes (`HardBreach`,
//! `RateExceeded`, `RotationConflict`) are surfaced inside the
//! returned [`control_plane_types`] mirror enums — NOT mapped to
//! `Err`. Callers match on the outcome.
//!
//! [`control_plane_types`]: super::control_plane_types
use async_trait::async_trait;
use ff_core::types::{BudgetId, ExecutionId, QuotaPolicyId};

use crate::error::FabricError;

use super::control_plane_types::{
    BudgetSpendOutcome, BudgetStatusSnapshot, CancelFlowInput, CancelRunInput, ClaimGrantOutcome,
    CompleteRunInput, CreateFlowInput, CreateRunExecutionInput, DeliverApprovalSignalInput,
    ExecutionCreated, FailExecutionOutcome, FailRunInput, FlowCancelOutcome,
    IssueGrantAndClaimInput, QuotaAdmission, ResumeRunInput, RotationOutcome, SuspendRunInput,
};

/// Cairn-side FCALL backend for budget, quota, and rotation
/// control-plane ops.
#[async_trait]
pub trait ControlPlaneBackend: Send + Sync {
    // ── Budget ───────────────────────────────────────────────────────────

    /// Create a budget scoped to `(scope_type, scope_id)` with the
    /// given dimension / hard-limit / soft-limit lanes.
    ///
    /// Validation on `scope_type` / `scope_id` (SEC-008: no control
    /// chars, no empty, ≤256 chars) is performed caller-side in
    /// [`FabricBudgetService`](crate::services::FabricBudgetService)
    /// before this method is invoked.
    #[allow(clippy::too_many_arguments)]
    async fn create_budget(
        &self,
        scope_type: &str,
        scope_id: &str,
        dimensions: &[&str],
        hard_limits: &[u64],
        soft_limits: &[u64],
        reset_interval_ms: u64,
        enforcement_mode: &str,
    ) -> Result<BudgetId, FabricError>;

    /// Record spend against a budget.
    ///
    /// `dedup_key` is REQUIRED — callers derive it from
    /// `(budget_id, execution_id, sorted deltas)` via
    /// `compute_spend_idempotency_key` and pass it here. The key
    /// prefix (`{b:N}` hash-tag) is the backend's responsibility.
    async fn record_spend(
        &self,
        budget_id: &BudgetId,
        execution_id: &ExecutionId,
        dimension_deltas: &[(&str, u64)],
        idempotency_key: &str,
    ) -> Result<BudgetSpendOutcome, FabricError>;

    /// Release (reset) a budget's usage counters.
    async fn release_budget(&self, budget_id: &BudgetId) -> Result<(), FabricError>;

    /// Read a budget's current definition + usage. Returns `Ok(None)`
    /// when the budget does not exist in FF.
    async fn get_budget_status(
        &self,
        budget_id: &BudgetId,
    ) -> Result<Option<BudgetStatusSnapshot>, FabricError>;

    // ── Quota ────────────────────────────────────────────────────────────

    /// Create a quota policy scoped to `(scope_type, scope_id)`.
    async fn create_quota_policy(
        &self,
        scope_type: &str,
        scope_id: &str,
        window_seconds: u64,
        max_requests_per_window: u64,
        max_concurrent: u64,
    ) -> Result<QuotaPolicyId, FabricError>;

    /// Check admission against a quota policy. The call records the
    /// admission on success, so it's a mutator.
    async fn check_admission(
        &self,
        quota_policy_id: &QuotaPolicyId,
        execution_id: &ExecutionId,
        window_seconds: u64,
        rate_limit: u64,
        concurrency_cap: u64,
    ) -> Result<QuotaAdmission, FabricError>;

    // ── Rotation ─────────────────────────────────────────────────────────

    /// Rotate the waitpoint HMAC signing kid across every execution
    /// partition.
    ///
    /// Partition fan-out is sequential and idempotent — re-running with
    /// the same `(new_kid, new_secret_hex)` converges via each
    /// partition's `noop` reply. Partial success is surfaced via
    /// [`RotationOutcome::failed`]; operators re-run with the same
    /// inputs once the underlying faults clear.
    async fn rotate_waitpoint_hmac(
        &self,
        new_kid: &str,
        new_secret_hex: &str,
        grace_ms: u64,
    ) -> RotationOutcome;

    // ── Run lifecycle (Phase D PR 2a) ───────────────────────────────────
    //
    // FCALL-driven transitions over an `ExecutionId` that represent a
    // cairn run's lifetime inside FF. Each method wraps ONE FCALL
    // (`ff_create_execution`, `ff_complete_execution`, …) with its
    // KEYS/ARGV layout. Services supply the pre-read `ExecutionLeaseContext`
    // — they already have it from `engine.describe_execution` — so the
    // impl never re-HGETALLs `exec_core`.
    //
    // Bridge-event emission stays caller-side: the service owns the
    // cairn-state-layer decision of which `BridgeEvent` to fire and
    // with what `prev_state`. This trait is FF-state-plane only.

    /// Create a run's FF execution. Idempotent on `execution_id`.
    async fn create_run_execution(
        &self,
        input: CreateRunExecutionInput,
    ) -> Result<ExecutionCreated, FabricError>;

    /// Mark an execution complete (terminal-success).
    async fn complete_run_execution(&self, input: CompleteRunInput) -> Result<(), FabricError>;

    /// Fail an execution. Returns the typed outcome — callers MUST
    /// branch on `RetryScheduled` vs `TerminalFailed` before emitting
    /// `BridgeEvent::ExecutionFailed`.
    async fn fail_run_execution(
        &self,
        input: FailRunInput,
    ) -> Result<FailExecutionOutcome, FabricError>;

    /// Cancel an execution (operator-initiated terminal).
    async fn cancel_run_execution(&self, input: CancelRunInput) -> Result<(), FabricError>;

    /// Suspend an execution. Shared by run-pause and
    /// enter-waiting-approval; the difference is entirely in the
    /// `SuspendRunInput` fields the caller fills in (reason_code,
    /// resume_condition_json, timeout_at).
    async fn suspend_run_execution(&self, input: SuspendRunInput) -> Result<(), FabricError>;

    /// Resume a suspended execution.
    async fn resume_run_execution(&self, input: ResumeRunInput) -> Result<(), FabricError>;

    /// Deliver an approval signal (approved / rejected) to a run's
    /// current waitpoint. Reads the HMAC waitpoint token from FF
    /// inline; callers never see the token.
    async fn deliver_approval_signal(
        &self,
        input: DeliverApprovalSignalInput,
    ) -> Result<(), FabricError>;

    // ── Session lifecycle (Phase D PR 2a) ───────────────────────────────

    /// Create a flow. Idempotent via FF's `ok_already_satisfied` reply
    /// — callers MUST emit their `BridgeEvent::SessionCreated`
    /// unconditionally (the read-model projection is idempotent on
    /// `EventId`; skipping on the retry would create a permanent
    /// projection gap when cairn crashed between FCALL commit and
    /// bridge emit).
    async fn create_flow(&self, input: CreateFlowInput) -> Result<(), FabricError>;

    /// Cancel (archive) a flow. Returns `AlreadyTerminal` when FF's
    /// Lua replies `flow_already_terminal` — idempotent re-archive.
    async fn cancel_flow(&self, input: CancelFlowInput) -> Result<FlowCancelOutcome, FabricError>;

    // ── Claim (Phase D PR 2a) ───────────────────────────────────────────

    /// Execute the `ff_issue_claim_grant` + `ff_claim_execution` pair.
    /// Dispatches transparently to `ff_claim_resumed_execution` when
    /// FF signals `use_claim_resumed_execution` (i.e. claim landed on
    /// an `attempt_interrupted` execution after a suspension resume).
    async fn issue_grant_and_claim(
        &self,
        input: IssueGrantAndClaimInput,
    ) -> Result<ClaimGrantOutcome, FabricError>;
}
