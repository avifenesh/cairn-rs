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
//! # Phase D PR 1 scope
//!
//! Budget (5 methods), quota (2 methods), rotation (1 method). Phase D
//! PR 2 adds run/task/session lifecycle FCALLs to this trait (or,
//! based on shape, a separate `ExecutionLifecycleBackend`).
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
    BudgetSpendOutcome, BudgetStatusSnapshot, QuotaAdmission, RotationOutcome,
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
}
