//! Cairn-native mirror types for the [`ControlPlaneBackend`] trait.
//!
//! These mirror the FF wire types that budget/quota/rotation/worker
//! services used to surface directly from FF (`ff_core::contracts::*`).
//! They exist so the [`ControlPlaneBackend`] trait boundary does not
//! leak FF-specific enums through to cairn services; when FF renames a
//! variant or reshapes the wire format, the mirror absorbs the change
//! and services stay unchanged.
//!
//! Phase D PR 1 introduces these alongside the trait. A small
//! conversion in `ValkeyControlPlane` translates FF's enum variants
//! (`ff_core::contracts::ReportUsageResult`, etc.) into the mirrors.
//!
//! [`ControlPlaneBackend`]: super::control_plane::ControlPlaneBackend
use std::collections::HashMap;

/// Result of a budget spend via
/// [`ControlPlaneBackend::record_spend`](super::control_plane::ControlPlaneBackend::record_spend).
///
/// Mirror of `ff_core::contracts::ReportUsageResult` with cairn-native
/// variant names. `SoftBreach` and `HardBreach` distinguish whether
/// the increment applied (`Soft` = applied + warning; `Hard` =
/// rejected).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BudgetSpendOutcome {
    /// All increments applied, no breach.
    Ok,
    /// Soft limit breached on a dimension (advisory, increments applied).
    SoftBreach {
        dimension: String,
        current_usage: u64,
        soft_limit: u64,
    },
    /// Hard limit breached (increments NOT applied).
    HardBreach {
        dimension: String,
        current_usage: u64,
        hard_limit: u64,
    },
    /// Dedup key matched — usage already applied in a prior call.
    AlreadyApplied,
}

/// Admission decision for a quota check via
/// [`ControlPlaneBackend::check_admission`](super::control_plane::ControlPlaneBackend::check_admission).
///
/// Mirror of the wire result returned by
/// `ff_check_admission_and_record`. `RateExceeded` carries a
/// retry-after hint the caller can surface as `Retry-After`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuotaAdmission {
    Admitted,
    AlreadyAdmitted,
    RateExceeded { retry_after_ms: u64 },
    ConcurrencyExceeded,
}

/// Snapshot of a budget's current definition + usage, returned by
/// [`ControlPlaneBackend::get_budget_status`](super::control_plane::ControlPlaneBackend::get_budget_status).
///
/// Previously `FabricBudgetService::BudgetStatus` — hoisted here so it
/// sits on the trait boundary rather than inside the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetStatusSnapshot {
    pub budget_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub enforcement_mode: String,
    pub usage: HashMap<String, u64>,
    pub hard_limits: HashMap<String, u64>,
    pub soft_limits: HashMap<String, u64>,
    pub breach_count: u64,
    pub soft_breach_count: u64,
}

/// Outcome of a waitpoint HMAC rotation fan-out via
/// [`ControlPlaneBackend::rotate_waitpoint_hmac`](super::control_plane::ControlPlaneBackend::rotate_waitpoint_hmac).
///
/// Count of partitions that rotated, no-op'd, and failed. Failures
/// carry an opaque classification hint only (see [`RotationFailure`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotationOutcome {
    /// Partitions that accepted a fresh rotation.
    pub rotated: u16,
    /// Partitions that replied `noop` (exact replay of same kid+secret).
    pub noop: u16,
    /// Per-partition failures.
    pub failed: Vec<RotationFailure>,
    /// Echoed input kid.
    pub new_kid: String,
}

/// Per-partition failure detail for the rotation fan-out.
///
/// SEC-007: only `code` and `partition_index` reach the HTTP response
/// body. Raw error strings / FCALL names / Valkey internals are logged
/// server-side but not carried here. `detail` is a classification hint
/// (`"lua_rejected"`, `"transport_error"`, `"unparseable_envelope"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotationFailure {
    pub partition_index: u16,
    /// FF typed error code when the Lua envelope returned one.
    /// `None` when the call failed before FCALL reply.
    pub code: Option<String>,
    /// Opaque classification hint. Does NOT contain raw error strings
    /// or FCALL names.
    pub detail: String,
}

/// Registration record returned by
/// [`Engine::register_worker`](super::Engine::register_worker).
///
/// Echoes the inputs plus the timestamp FF stamped on the hash so the
/// caller can log or surface it without re-reading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerRegistration {
    pub worker_id: ff_core::types::WorkerId,
    pub instance_id: ff_core::types::WorkerInstanceId,
    pub capabilities: Vec<String>,
    pub registered_at_ms: u64,
}

// ── Phase D PR 2a: run / session / claim lifecycle mirrors ──────────────

/// Result of a `create_run_execution` call.
///
/// FF's `ff_create_execution` is idempotent on `execution_id`: a second
/// call with the same id returns a `DUPLICATE` envelope and no state
/// changes. Callers rely on `newly_created` to decide whether to emit
/// `BridgeEvent::ExecutionCreated` exactly once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionCreated {
    pub newly_created: bool,
}

/// Outcome of [`ControlPlaneBackend::fail_run_execution`].
///
/// Mirror of the internal `helpers::FailOutcome` at the trait boundary
/// so services never see the FF envelope classification directly. FF
/// returns `retry_scheduled` when its retry policy schedules another
/// attempt rather than terminating the execution; callers MUST NOT emit
/// `BridgeEvent::ExecutionFailed` on the retry path (that would
/// project a terminal state onto a still-running execution — exactly
/// the silent-emission class of bug Phase B called out).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailExecutionOutcome {
    RetryScheduled,
    TerminalFailed,
}

/// Outcome of [`ControlPlaneBackend::cancel_flow`].
///
/// `AlreadyTerminal` is the idempotent re-archive / re-cancel path —
/// FF's Lua returns `flow_already_terminal` when the flow is already
/// completed/cancelled. Cairn still needs to stamp `cairn.archived`
/// on a terminal flow for list-filtering semantics, so the service
/// treats this variant as success (not error).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowCancelOutcome {
    Cancelled,
    AlreadyTerminal,
}

/// Result of a successful claim through
/// [`ControlPlaneBackend::issue_grant_and_claim`].
///
/// Cairn does NOT consume the lease triple — every downstream terminal
/// op re-reads `current_lease_id` / `_epoch` / `_attempt_index` from
/// FF's `exec_core` on demand. Keeping this struct carries the FCALL's
/// typed response without a cairn cache; the fields are read via
/// tests / debug logs only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimGrantOutcome {
    pub lease_id: ff_core::types::LeaseId,
    pub lease_epoch: ff_core::types::LeaseEpoch,
    pub attempt_index: ff_core::types::AttemptIndex,
}

// ── Input structs ───────────────────────────────────────────────────────
//
// Typed cairn-native inputs for the lifecycle FCALLs. The trait impl
// builds FF key contexts + ARGV from these — the fields that come
// from a prior `describe_execution` snapshot are supplied explicitly
// by the caller, so the impl never has to re-read.

/// Snapshot fields a lifecycle FCALL needs (lease triple + attempt
/// pointer + lane + worker identity). Populated by the service from
/// an `ExecutionSnapshot` before the FCALL.
#[derive(Clone, Debug)]
pub struct ExecutionLeaseContext {
    pub lane_id: ff_core::types::LaneId,
    pub attempt_index: ff_core::types::AttemptIndex,
    pub lease_id: String,
    pub lease_epoch: String,
    pub attempt_id: String,
    pub worker_instance_id: ff_core::types::WorkerInstanceId,
}

/// Input to `create_run_execution`.
#[derive(Clone, Debug)]
pub struct CreateRunExecutionInput {
    pub execution_id: ff_core::types::ExecutionId,
    pub namespace: ff_core::types::Namespace,
    pub lane_id: ff_core::types::LaneId,
    /// `cairn.*` tags to stamp on `exec_tags`. Caller owns the full
    /// set (run_id / session_id / project / instance_id / optional
    /// parent + correlation).
    pub tags: std::collections::HashMap<String, String>,
    /// JSON-encoded retry policy. Empty string means no policy.
    pub policy_json: String,
}

/// Input to `complete_run_execution`.
#[derive(Clone, Debug)]
pub struct CompleteRunInput {
    pub execution_id: ff_core::types::ExecutionId,
    pub lease: ExecutionLeaseContext,
}

/// Input to `cancel_run_execution`.
#[derive(Clone, Debug)]
pub struct CancelRunInput {
    pub execution_id: ff_core::types::ExecutionId,
    pub lease: ExecutionLeaseContext,
    /// Current waitpoint id on the execution, if any. Empty means no
    /// active waitpoint (FF's cancel tolerates a default/empty id).
    pub current_waitpoint: Option<ff_core::types::WaitpointId>,
}

/// Input to `fail_run_execution`.
#[derive(Clone, Debug)]
pub struct FailRunInput {
    pub execution_id: ff_core::types::ExecutionId,
    pub lease: ExecutionLeaseContext,
    pub reason: String,
    pub category: String,
    /// JSON-encoded retry policy read from `exec_policy`. Empty means
    /// "no policy" — FF falls back to its default.
    pub retry_policy_json: String,
}

/// A suspension request built by the caller from a `SuspensionParams`.
///
/// The service assembles the resume-condition / resume-policy JSON +
/// timeout-at calculations; the trait impl only wires them into the
/// `ff_suspend_execution` KEYS/ARGV layout.
#[derive(Clone, Debug)]
pub struct SuspendRunInput {
    pub execution_id: ff_core::types::ExecutionId,
    pub lease: ExecutionLeaseContext,
    pub reason_code: String,
    pub timeout_at: String,
    pub resume_condition_json: String,
    pub resume_policy_json: String,
    pub timeout_behavior: String,
}

/// Input to `resume_run_execution`.
#[derive(Clone, Debug)]
pub struct ResumeRunInput {
    pub execution_id: ff_core::types::ExecutionId,
    pub lane_id: ff_core::types::LaneId,
    pub waitpoint_id: Option<ff_core::types::WaitpointId>,
    pub resume_source: String,
}

/// Input to `deliver_approval_signal`.
#[derive(Clone, Debug)]
pub struct DeliverApprovalSignalInput {
    pub execution_id: ff_core::types::ExecutionId,
    pub lane_id: ff_core::types::LaneId,
    pub waitpoint_id: ff_core::types::WaitpointId,
    pub signal_name: String,
    pub idempotency_suffix: String,
    pub signal_dedup_ttl_ms: u64,
    pub maxlen: u64,
    pub max_signals_per_execution: u64,
}

/// Input to `create_flow`.
#[derive(Clone, Debug)]
pub struct CreateFlowInput {
    pub flow_id: ff_core::types::FlowId,
    pub flow_kind: String,
    pub namespace: ff_core::types::Namespace,
}

/// Input to `cancel_flow`.
#[derive(Clone, Debug)]
pub struct CancelFlowInput {
    pub flow_id: ff_core::types::FlowId,
    pub reason: String,
    pub cancel_mode: String,
}

/// Input to `issue_grant_and_claim`.
#[derive(Clone, Debug)]
pub struct IssueGrantAndClaimInput {
    pub execution_id: ff_core::types::ExecutionId,
    pub lane_id: ff_core::types::LaneId,
    pub lease_duration_ms: u64,
}
