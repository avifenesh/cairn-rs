//! RFC 014 — entitlement / feature-gate end-to-end integration tests.
//!
//! Verifies the full lifecycle of the feature-gate system through the
//! `LicenseService` boundary:
//!
//!   1. No license  → every feature is Denied (fail-closed)
//!   2. LocalEval   → GA features (e.g. `runtime_core`) are Allowed;
//!                    entitlement-gated features remain Denied
//!   3. Unknown feature names → always Denied (fail-closed, RFC 014 §4)
//!   4. Operator overrides can unblock a denied feature per-tenant
//!   5. Overrides can also block an otherwise-GA feature per-tenant

use std::sync::Arc;

use cairn_domain::{FeatureGateResult, ProductTier, TenantId};
use cairn_runtime::licenses::LicenseService;
use cairn_runtime::services::LicenseServiceImpl;
use cairn_store::InMemoryStore;

fn tenant(id: &str) -> TenantId {
    TenantId::new(id)
}

// ── Test 1: no license → everything denied ───────────────────────────────────

/// RFC 014 §3: a tenant without any license must be denied access to every
/// feature, including GA features.  The gate must be fail-closed by default.
#[tokio::test]
async fn no_license_denies_all_features() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LicenseServiceImpl::new(store);
    let tid = tenant("t_no_license");

    // No license has been activated — get_active must return None.
    let active = svc.get_active(&tid).await.unwrap();
    assert!(active.is_none(), "no license must be present before activation");

    // GA features denied without a license.
    let result = svc.check_feature(&tid, "runtime_core").await.unwrap();
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "RFC 014: runtime_core must be Denied when no license is active; got: {result:?}"
    );

    // Entitlement-gated features also denied.
    let result = svc.check_feature(&tid, "advanced_audit_export").await.unwrap();
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "RFC 014: advanced_audit_export must be Denied when no license is active"
    );

    // Reason string must mention the tenant.
    if let FeatureGateResult::Denied { reason } = svc.check_feature(&tid, "runtime_core").await.unwrap() {
        assert!(
            reason.contains("t_no_license"),
            "denial reason must reference the tenant ID; got: '{reason}'"
        );
    }
}

// ── Test 2: LocalEval license → GA features allowed, gated features denied ───

/// RFC 014 §3: a LocalEval license grants access to GA features (`runtime_core`,
/// `retrieval_core`) but must not grant entitlement-gated features.
#[tokio::test]
async fn local_eval_license_allows_ga_features_only() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LicenseServiceImpl::new(store);
    let tid = tenant("t_local_eval");

    svc.activate(tid.clone(), ProductTier::LocalEval, None)
        .await
        .unwrap();

    // GA feature → Allowed.
    let result = svc.check_feature(&tid, "runtime_core").await.unwrap();
    assert_eq!(
        result,
        FeatureGateResult::Allowed,
        "RFC 014: runtime_core must be Allowed with a LocalEval license"
    );

    // Second GA feature → Allowed.
    let result = svc.check_feature(&tid, "retrieval_core").await.unwrap();
    assert_eq!(
        result,
        FeatureGateResult::Allowed,
        "RFC 014: retrieval_core must be Allowed with a LocalEval license"
    );

    // Governance-gated feature → Denied (LocalEval has no GovernanceCompliance entitlement).
    let result = svc.check_feature(&tid, "advanced_audit_export").await.unwrap();
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "RFC 014: advanced_audit_export must be Denied on LocalEval (no GovernanceCompliance)"
    );

    let result = svc.check_feature(&tid, "compliance_policy_packs").await.unwrap();
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "RFC 014: compliance_policy_packs must be Denied on LocalEval"
    );

    let result = svc.check_feature(&tid, "approval_hardening").await.unwrap();
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "RFC 014: approval_hardening must be Denied on LocalEval"
    );

    let result = svc.check_feature(&tid, "advanced_admin").await.unwrap();
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "RFC 014: advanced_admin must be Denied on LocalEval (no AdvancedAdmin entitlement)"
    );
}

// ── Test 3: unknown features → always denied (fail-closed) ───────────────────

/// RFC 014 §4: unrecognized feature names must be Denied, never silently
/// allowed.  Fail-closed is mandatory — defaulting unknown features to Allowed
/// would grant access to anything not explicitly listed.
#[tokio::test]
async fn unknown_feature_always_denied_fail_closed() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LicenseServiceImpl::new(store);
    let tid = tenant("t_unknown_feature");

    // Without any license.
    let result = svc.check_feature(&tid, "totally_made_up_feature").await.unwrap();
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "RFC 014: unknown feature must be Denied with no license"
    );

    // Even with an active license — unknown features must still be Denied.
    svc.activate(tid.clone(), ProductTier::LocalEval, None)
        .await
        .unwrap();

    let result = svc.check_feature(&tid, "totally_made_up_feature").await.unwrap();
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "RFC 014: unknown feature must be Denied even with an active license"
    );

    let result = svc.check_feature(&tid, "").await.unwrap();
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "RFC 014: empty feature string must be Denied"
    );

    // Denial reason must mention the feature name (not a generic error).
    if let FeatureGateResult::Denied { reason } = svc.check_feature(&tid, "ghost_capability").await.unwrap() {
        assert!(
            reason.contains("ghost_capability"),
            "denial reason must name the unrecognized feature; got: '{reason}'"
        );
    }
}

// ── Test 4: operator override unblocks a denied entitlement-gated feature ────

/// RFC 014 §5: an operator override with `allowed = true` must promote a
/// Denied feature to Allowed regardless of the tenant's tier / entitlements.
#[tokio::test]
async fn operator_override_unblocks_entitlement_gated_feature() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LicenseServiceImpl::new(store);
    let tid = tenant("t_override_allow");

    svc.activate(tid.clone(), ProductTier::LocalEval, None)
        .await
        .unwrap();

    // Baseline: feature is Denied.
    let before = svc.check_feature(&tid, "advanced_audit_export").await.unwrap();
    assert!(
        matches!(before, FeatureGateResult::Denied { .. }),
        "baseline must be Denied before override"
    );

    // Apply allow override.
    let record = svc
        .set_override(
            tid.clone(),
            "advanced_audit_export".to_owned(),
            true,
            Some("sales override for pilot".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(record.feature, "advanced_audit_export");
    assert!(record.allowed);

    // Feature must now be Allowed.
    let after = svc.check_feature(&tid, "advanced_audit_export").await.unwrap();
    assert_eq!(
        after,
        FeatureGateResult::Allowed,
        "RFC 014: feature must be Allowed after operator override"
    );

    // Override must not affect a different tenant.
    let tid2 = tenant("t_override_isolation");
    svc.activate(tid2.clone(), ProductTier::LocalEval, None)
        .await
        .unwrap();
    let isolated = svc.check_feature(&tid2, "advanced_audit_export").await.unwrap();
    assert!(
        matches!(isolated, FeatureGateResult::Denied { .. }),
        "RFC 014: override must be scoped to the tenant; other tenants must remain Denied"
    );
}

// ── Test 5: operator override can block a GA feature ─────────────────────────

/// RFC 014 §5: an operator override with `allowed = false` must block a
/// feature that would otherwise be Allowed, enabling controlled rollouts and
/// emergency kill-switches.
#[tokio::test]
async fn operator_override_blocks_ga_feature() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LicenseServiceImpl::new(store);
    let tid = tenant("t_override_deny");

    svc.activate(tid.clone(), ProductTier::LocalEval, None)
        .await
        .unwrap();

    // Baseline: GA feature is Allowed.
    let before = svc.check_feature(&tid, "runtime_core").await.unwrap();
    assert_eq!(before, FeatureGateResult::Allowed, "baseline must be Allowed");

    // Apply deny override with a reason.
    svc.set_override(
        tid.clone(),
        "runtime_core".to_owned(),
        false,
        Some("maintenance window".to_owned()),
    )
    .await
    .unwrap();

    // Feature must now be Denied, and the reason must be surfaced.
    let after = svc.check_feature(&tid, "runtime_core").await.unwrap();
    match after {
        FeatureGateResult::Denied { reason } => {
            assert!(
                reason.contains("maintenance window"),
                "RFC 014: override reason must propagate to denial message; got: '{reason}'"
            );
        }
        other => panic!("RFC 014: runtime_core must be Denied after deny override; got: {other:?}"),
    }
}

// ── Test 6: multiple tenants are fully isolated ────────────────────────────────

/// RFC 014 §3: each tenant has an independent license and entitlement set.
/// Licenses and overrides for one tenant must never affect another.
#[tokio::test]
async fn tenant_license_isolation() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LicenseServiceImpl::new(store);

    let t_licensed = tenant("t_licensed");
    let t_unlicensed = tenant("t_unlicensed");

    svc.activate(t_licensed.clone(), ProductTier::LocalEval, None)
        .await
        .unwrap();

    // t_licensed: GA feature Allowed.
    let r1 = svc.check_feature(&t_licensed, "runtime_core").await.unwrap();
    assert_eq!(r1, FeatureGateResult::Allowed);

    // t_unlicensed: same feature Denied — no license activated.
    let r2 = svc.check_feature(&t_unlicensed, "runtime_core").await.unwrap();
    assert!(
        matches!(r2, FeatureGateResult::Denied { .. }),
        "RFC 014: t_unlicensed must not inherit t_licensed's license"
    );
}
