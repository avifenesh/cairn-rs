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
