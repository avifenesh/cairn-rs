//! Budget service — thin shim over [`ControlPlaneBackend`].
//!
//! As of Phase D PR 1, all FF-shaped logic (key builders, FCALL
//! dispatch, envelope parsing) lives in the backend impl (see
//! `engine/valkey_control_plane_impl.rs`). This service keeps only:
//!
//!   * SEC-008 scope-field validation (control-char + length guard)
//!     that runs BEFORE the backend call — the backend's FCALL never
//!     sees a hostile `scope_type` / `scope_id`.
//!   * Idempotency-key derivation (`compute_spend_idempotency_key`) —
//!     pure, deterministic, lives caller-side so test + production
//!     paths share exactly one implementation.
//!   * Convenience constructors (`create_run_budget`, …) that pin the
//!     standard `(tokens, cost_microdollars, api_calls)` dimensions.
//!
//! **Lean-bridge silence (intentional).** None of this service's
//! methods emit `BridgeEvent`s — budget state is FF-owned operational
//! state with no `BudgetReadModel` projection on the cairn-store side.
//! `record_spend` is additionally volume-sensitive — it fires on every
//! tool call / LLM token charge. See `docs/design/bridge-event-audit.md`
//! §2.6 for the full rationale.
use std::sync::Arc;

use ff_core::types::{BudgetId, ExecutionId};
use uuid::Uuid;

use crate::engine::control_plane::ControlPlaneBackend;
use crate::engine::control_plane_types::{BudgetSpendOutcome, BudgetStatusSnapshot};
use crate::error::FabricError;

/// Re-export of the snapshot type as the historical service-level
/// name. Keeps downstream code that imported
/// `crate::services::budget_service::BudgetStatus` working.
pub type BudgetStatus = BudgetStatusSnapshot;

// Stable namespace UUID for spend-dedup keys. Mirrors the UUID v5 +
// null-byte-delimited scheme from id_map.rs. Changing these bytes orphans any
// in-flight idempotency record.
const SPEND_NAMESPACE: Uuid = Uuid::from_bytes([
    0xb7, 0x1a, 0x2e, 0x04, 0x9c, 0x85, 0x45, 0xc3, 0x88, 0xd9, 0x0f, 0x4a, 0x6b, 0x2c, 0x13, 0x77,
]);

const SPEND_NAMESPACE_VERSION: u8 = 1;

/// Maximum byte length for `scope_type` / `scope_id` fields passed into
/// `ff_create_budget`. Matches `cairn-app::validate::MAX_ID_LEN` (128) ×2 to
/// leave headroom for scoped identifiers without hard-coding a cross-crate
/// constant dependency.
const SCOPE_FIELD_MAX_LEN: usize = 256;

/// Validate a scope field (scope_type / scope_id) passed to `create_budget`.
///
/// SEC-008 guard: cairn-fabric does not import cairn-app's `validate`
/// module, but it still must reject control-character and oversized inputs
/// before they reach FF's Valkey key builders (where they become opaque
/// hash-tag components). Empty / zero-length rejected so a blank scope
/// never silently collides with a legitimate bootstrap "default".
pub(crate) fn validate_scope_field(field: &str, name: &str) -> Result<(), FabricError> {
    if field.is_empty() {
        return Err(FabricError::Validation {
            reason: format!("{name} is empty"),
        });
    }
    if field.len() > SCOPE_FIELD_MAX_LEN {
        return Err(FabricError::Validation {
            reason: format!("{name} exceeds {SCOPE_FIELD_MAX_LEN} chars"),
        });
    }
    if field.chars().any(|c| c.is_control()) {
        return Err(FabricError::Validation {
            reason: format!("{name} contains control characters"),
        });
    }
    Ok(())
}

/// Derive a stable idempotency key for a spend call.
///
/// Stable across retries for the same (budget, execution, dimension set/amount).
/// Callers that repeat a spend with identical inputs produce an identical key,
/// so FF dedups server-side via the `dedup_key` ARGV slot of
/// `ff_report_usage_and_check`.
///
/// Scheme: UUID v5 over `"v{ver}:spend:\0{budget}\0{execution}\0{sorted dim\0delta pairs}"`.
/// Null-byte delimiters match id_map.rs and eliminate colon-boundary collisions
/// (e.g. dim "a:b" vs dims "a"+"b").
pub(crate) fn compute_spend_idempotency_key(
    budget_id: &BudgetId,
    execution_id: &ExecutionId,
    dimension_deltas: &[(&str, u64)],
) -> String {
    let mut sorted: Vec<(&str, u64)> = dimension_deltas.to_vec();
    sorted.sort_by_key(|r| r.0);

    let mut input = format!("v{SPEND_NAMESPACE_VERSION}:spend:\0{budget_id}\0{execution_id}");
    for (dim, delta) in &sorted {
        input.push('\0');
        input.push_str(dim);
        input.push('\0');
        input.push_str(&delta.to_string());
    }
    Uuid::new_v5(&SPEND_NAMESPACE, input.as_bytes()).to_string()
}

/// Budget service — public surface for cairn-app / cairn-runtime.
pub struct FabricBudgetService {
    backend: Arc<dyn ControlPlaneBackend>,
}

impl FabricBudgetService {
    pub fn new(backend: Arc<dyn ControlPlaneBackend>) -> Self {
        Self { backend }
    }

    pub async fn create_budget(
        &self,
        scope_type: &str,
        scope_id: &str,
        dimensions: &[&str],
        hard_limits: &[u64],
        soft_limits: &[u64],
        reset_interval_ms: u64,
        enforcement_mode: &str,
    ) -> Result<BudgetId, FabricError> {
        // SEC-008: reject control characters / empty / oversized scope
        // inputs before they flow into FF key builders.
        validate_scope_field(scope_type, "scope_type")?;
        validate_scope_field(scope_id, "scope_id")?;

        self.backend
            .create_budget(
                scope_type,
                scope_id,
                dimensions,
                hard_limits,
                soft_limits,
                reset_interval_ms,
                enforcement_mode,
            )
            .await
    }

    pub async fn create_run_budget(
        &self,
        run_id: &cairn_domain::RunId,
        token_limit: u64,
        cost_limit_microdollars: u64,
        api_call_limit: u64,
    ) -> Result<BudgetId, FabricError> {
        self.create_budget(
            "run",
            run_id.as_str(),
            &["tokens", "cost_microdollars", "api_calls"],
            &[token_limit, cost_limit_microdollars, api_call_limit],
            &[
                token_limit * 80 / 100,
                cost_limit_microdollars * 80 / 100,
                api_call_limit * 80 / 100,
            ],
            0,
            "enforce",
        )
        .await
    }

    pub async fn create_tenant_budget(
        &self,
        tenant_id: &cairn_domain::TenantId,
        token_limit: u64,
        cost_limit_microdollars: u64,
        api_call_limit: u64,
        reset_interval_ms: u64,
    ) -> Result<BudgetId, FabricError> {
        self.create_budget(
            "tenant",
            tenant_id.as_str(),
            &["tokens", "cost_microdollars", "api_calls"],
            &[token_limit, cost_limit_microdollars, api_call_limit],
            &[
                token_limit * 80 / 100,
                cost_limit_microdollars * 80 / 100,
                api_call_limit * 80 / 100,
            ],
            reset_interval_ms,
            "enforce",
        )
        .await
    }

    pub async fn release_budget(&self, budget_id: &BudgetId) -> Result<(), FabricError> {
        self.backend.release_budget(budget_id).await
    }

    /// Record spend against a budget. Returns the cairn-native
    /// [`BudgetSpendOutcome`] — callers match on the variants directly.
    ///
    /// `execution_id` is REQUIRED. Two calls that share an idempotency
    /// key are treated by FF as the same spend (the second returns
    /// [`BudgetSpendOutcome::AlreadyApplied`] and the budget is not
    /// double-decremented). The key's caller-identity component comes
    /// from the ExecutionId, so every distinct logical spend MUST
    /// present a distinct ExecutionId.
    pub async fn record_spend(
        &self,
        budget_id: &BudgetId,
        execution_id: &ExecutionId,
        dimension_deltas: &[(&str, u64)],
    ) -> Result<BudgetSpendOutcome, FabricError> {
        let idempotency_key =
            compute_spend_idempotency_key(budget_id, execution_id, dimension_deltas);
        self.backend
            .record_spend(budget_id, execution_id, dimension_deltas, &idempotency_key)
            .await
    }

    pub async fn get_budget_status(
        &self,
        budget_id: &BudgetId,
    ) -> Result<BudgetStatus, FabricError> {
        match self.backend.get_budget_status(budget_id).await? {
            Some(status) => Ok(status),
            None => Err(FabricError::NotFound {
                entity: "budget",
                id: budget_id.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_eid;

    #[test]
    fn idempotency_key_stable_for_same_inputs() {
        let bid = BudgetId::new();
        let eid = test_eid("stable");
        let k1 = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 50)]);
        let k2 = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 50)]);
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 36);
    }

    #[test]
    fn idempotency_key_differs_when_inputs_change() {
        let bid = BudgetId::new();
        let eid = test_eid("differs");
        let k_tokens = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 50)]);
        let k_cost = compute_spend_idempotency_key(&bid, &eid, &[("cost", 50)]);
        let k_amount = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 51)]);
        assert_ne!(k_tokens, k_cost);
        assert_ne!(k_tokens, k_amount);
    }

    #[test]
    fn idempotency_key_order_independent_for_same_dimension_set() {
        let bid = BudgetId::new();
        let eid = test_eid("order");
        let k_ab = compute_spend_idempotency_key(&bid, &eid, &[("a", 1), ("b", 2)]);
        let k_ba = compute_spend_idempotency_key(&bid, &eid, &[("b", 2), ("a", 1)]);
        assert_eq!(k_ab, k_ba);
    }

    #[test]
    fn idempotency_key_isolates_execution() {
        let bid = BudgetId::new();
        let eid1 = test_eid("exec1");
        let eid2 = test_eid("exec2");
        let k1 = compute_spend_idempotency_key(&bid, &eid1, &[("tokens", 50)]);
        let k2 = compute_spend_idempotency_key(&bid, &eid2, &[("tokens", 50)]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn idempotency_key_isolates_budget() {
        let b1 = BudgetId::new();
        let b2 = BudgetId::new();
        let eid = test_eid("budget_iso");
        let k1 = compute_spend_idempotency_key(&b1, &eid, &[("tokens", 50)]);
        let k2 = compute_spend_idempotency_key(&b2, &eid, &[("tokens", 50)]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn idempotency_key_no_delimiter_collision() {
        let bid = BudgetId::new();
        let eid = test_eid("delim");
        let k1 = compute_spend_idempotency_key(&bid, &eid, &[("a:b", 1), ("c", 2)]);
        let k2 = compute_spend_idempotency_key(&bid, &eid, &[("a", 1), ("b:c", 2)]);
        assert_ne!(k1, k2);
    }

    // ── SEC-008 validate_scope_field ───────────────────────────────────

    #[test]
    fn validate_scope_field_rejects_empty() {
        let err = validate_scope_field("", "scope_id").unwrap_err();
        match err {
            FabricError::Validation { reason } => assert!(reason.contains("empty")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn validate_scope_field_rejects_control_chars() {
        let err = validate_scope_field("run\x01id", "scope_id").unwrap_err();
        match err {
            FabricError::Validation { reason } => assert!(reason.contains("control characters")),
            other => panic!("expected Validation, got {other:?}"),
        }
        let err = validate_scope_field("run\x00id", "scope_id").unwrap_err();
        assert!(matches!(err, FabricError::Validation { .. }));
    }

    #[test]
    fn validate_scope_field_rejects_oversized() {
        let long = "x".repeat(SCOPE_FIELD_MAX_LEN + 1);
        let err = validate_scope_field(&long, "scope_id").unwrap_err();
        match err {
            FabricError::Validation { reason } => assert!(reason.contains("exceeds")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn validate_scope_field_accepts_at_max_len() {
        let exact = "a".repeat(SCOPE_FIELD_MAX_LEN);
        assert!(validate_scope_field(&exact, "scope_id").is_ok());
    }

    #[test]
    fn validate_scope_field_accepts_normal_id() {
        assert!(validate_scope_field("run_abc_123", "scope_id").is_ok());
        assert!(validate_scope_field("run", "scope_type").is_ok());
        assert!(validate_scope_field("a:b/c", "scope_id").is_ok());
    }

    #[test]
    fn standard_dimensions() {
        let dims = ["tokens", "cost_microdollars", "api_calls"];
        assert_eq!(dims.len(), 3);
        assert!(dims.contains(&"tokens"));
        assert!(dims.contains(&"cost_microdollars"));
        assert!(dims.contains(&"api_calls"));
    }
}
