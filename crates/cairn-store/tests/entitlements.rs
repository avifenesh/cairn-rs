//! RFC 014 commercial entitlements integration tests.
//!
//! Validates the commercial gating pipeline through InMemoryStore:
//! - LicenseActivated event projects into the license read-model.
//! - EntitlementOverrideSet event projects into the override read-model.
//! - DefaultFeatureGate is fail-closed: unknown features return Denied.
//! - GA features are always Allowed regardless of entitlements.
//! - Entitlement-gated features are Allowed only when the entitlement is active.

use std::sync::Arc;

use cairn_domain::{
    commercial::{
        DefaultFeatureGate, Entitlement, EntitlementSet, FeatureGate, FeatureGateResult,
        ProductTier,
    },
    EntitlementOverrideSet, EventEnvelope, EventId, EventSource, LicenseActivated,
    RuntimeEvent, TenantId,
};
use cairn_store::{projections::LicenseReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn tenant_id() -> TenantId {
    TenantId::new("tenant_ent")
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(id),
        EventSource::System,
        payload.into(),
    )
}

fn license_activated_event(tier: ProductTier) -> EventEnvelope<RuntimeEvent> {
    ev(
        "evt_license_activated",
        RuntimeEvent::LicenseActivated(LicenseActivated {
            tenant_id: tenant_id(),
            license_id: "lic_enterprise_001".to_owned(),
            tier,
            valid_from_ms: 1_000_000,
            valid_until_ms: Some(2_000_000),
        }),
    )
}

fn override_event(feature: &str, allowed: bool) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_override_{feature}"),
        RuntimeEvent::EntitlementOverrideSet(EntitlementOverrideSet {
            tenant_id: tenant_id(),
            feature: feature.to_owned(),
            allowed,
            reason: Some(format!("operator override: {feature} = {allowed}")),
            set_at_ms: 1_500_000,
        }),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) Append LicenseActivated; (2) verify license read-model is queryable.
#[tokio::test]
async fn license_activated_is_stored_and_queryable() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[license_activated_event(ProductTier::EnterpriseSelfHosted)])
        .await
        .unwrap();

    let license = LicenseReadModel::get_active(store.as_ref(), &tenant_id())
        .await
        .unwrap()
        .expect("license must be present after LicenseActivated event");

    assert_eq!(license.tenant_id, tenant_id());
    assert_eq!(license.tier, ProductTier::EnterpriseSelfHosted);
    assert_eq!(
        license.license_key.as_deref(),
        Some("lic_enterprise_001"),
        "license key must be preserved"
    );
    assert_eq!(license.issued_at, 1_000_000);
    assert_eq!(license.expires_at, Some(2_000_000));
}

/// Absence of a LicenseActivated event means no license record exists.
#[tokio::test]
async fn no_license_when_none_activated() {
    let store = Arc::new(InMemoryStore::new());

    let license = LicenseReadModel::get_active(store.as_ref(), &tenant_id())
        .await
        .unwrap();
    assert!(license.is_none(), "no license should exist before LicenseActivated is appended");
}

/// (3) Append EntitlementOverrideSet; (4) verify override is active in read-model.
#[tokio::test]
async fn entitlement_override_is_stored_and_queryable() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            license_activated_event(ProductTier::TeamSelfHosted),
            override_event("advanced_audit_export", true),
        ])
        .await
        .unwrap();

    let overrides = LicenseReadModel::list_overrides(store.as_ref(), &tenant_id())
        .await
        .unwrap();

    assert_eq!(overrides.len(), 1, "one override must be stored");
    let override_rec = &overrides[0];
    assert_eq!(override_rec.tenant_id, tenant_id());
    assert_eq!(override_rec.feature, "advanced_audit_export");
    assert!(override_rec.granted, "override must be marked granted=true");
    assert!(override_rec.reason.is_some(), "override reason must be preserved");
}

/// Multiple overrides for the same tenant are all listed.
#[tokio::test]
async fn multiple_overrides_all_listed() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            license_activated_event(ProductTier::EnterpriseSelfHosted),
            override_event("advanced_audit_export", true),
            override_event("approval_hardening", true),
            override_event("advanced_admin", false),
        ])
        .await
        .unwrap();

    let overrides = LicenseReadModel::list_overrides(store.as_ref(), &tenant_id())
        .await
        .unwrap();

    assert_eq!(overrides.len(), 3, "all three overrides must be stored");

    let granted: Vec<_> = overrides.iter().filter(|o| o.granted).collect();
    let denied: Vec<_> = overrides.iter().filter(|o| !o.granted).collect();
    assert_eq!(granted.len(), 2, "two granted overrides");
    assert_eq!(denied.len(), 1, "one denied override");
}

/// (5) DefaultFeatureGate returns Denied for unknown feature names (fail-closed).
///
/// RFC 014: an unrecognized feature name MUST NOT be silently allowed.
/// Defaulting unknown features to Allowed would grant access to anything
/// not explicitly listed — the opposite of a secure-by-default posture.
#[tokio::test]
async fn unknown_feature_is_denied_fail_closed() {
    let gate = DefaultFeatureGate::v1_defaults();
    let entitlements = EntitlementSet::new(tenant_id(), ProductTier::EnterpriseSelfHosted);

    let result = gate.check(&entitlements, "totally_unknown_feature_xyz");

    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "unknown feature must return Denied (fail-closed), got: {result:?}"
    );

    // Another unknown feature variant.
    let result2 = gate.check(&entitlements, "");
    assert!(
        matches!(result2, FeatureGateResult::Denied { .. }),
        "empty feature name must also return Denied"
    );
}

/// (6) Known GA features always return Allowed, regardless of entitlements.
#[tokio::test]
async fn ga_features_always_allowed() {
    let gate = DefaultFeatureGate::v1_defaults();

    // LocalEval tenant with no entitlements should still access GA features.
    let minimal_entitlements = EntitlementSet::new(tenant_id(), ProductTier::LocalEval);

    for feature in &["runtime_core", "retrieval_core"] {
        let result = gate.check(&minimal_entitlements, feature);
        assert_eq!(
            result,
            FeatureGateResult::Allowed,
            "GA feature '{feature}' must be Allowed for any tier"
        );
    }
}

/// Entitlement-gated features are Allowed only when the required entitlement is present.
#[tokio::test]
async fn entitlement_gated_feature_allowed_with_entitlement() {
    let gate = DefaultFeatureGate::v1_defaults();

    // GovernanceCompliance entitlement required for advanced_audit_export.
    let entitlements = EntitlementSet::new(tenant_id(), ProductTier::EnterpriseSelfHosted)
        .with_entitlement(Entitlement::GovernanceCompliance);

    let result = gate.check(&entitlements, "advanced_audit_export");
    assert_eq!(
        result,
        FeatureGateResult::Allowed,
        "advanced_audit_export must be Allowed when GovernanceCompliance entitlement is present"
    );

    let result2 = gate.check(&entitlements, "compliance_policy_packs");
    assert_eq!(
        result2,
        FeatureGateResult::Allowed,
        "compliance_policy_packs must be Allowed with GovernanceCompliance"
    );

    let result3 = gate.check(&entitlements, "approval_hardening");
    assert_eq!(
        result3,
        FeatureGateResult::Allowed,
        "approval_hardening must be Allowed with GovernanceCompliance"
    );
}

/// Entitlement-gated features are Denied when the required entitlement is absent.
#[tokio::test]
async fn entitlement_gated_feature_denied_without_entitlement() {
    let gate = DefaultFeatureGate::v1_defaults();

    // No entitlements — all gated features must be Denied.
    let bare_entitlements = EntitlementSet::new(tenant_id(), ProductTier::TeamSelfHosted);

    for feature in &["advanced_audit_export", "compliance_policy_packs", "approval_hardening"] {
        let result = gate.check(&bare_entitlements, feature);
        assert!(
            matches!(result, FeatureGateResult::Denied { .. }),
            "entitlement-gated feature '{feature}' must be Denied without the required entitlement, got: {result:?}"
        );
    }

    // AdvancedAdmin entitlement required for advanced_admin.
    let result = gate.check(&bare_entitlements, "advanced_admin");
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "advanced_admin must be Denied without AdvancedAdmin entitlement"
    );
}

/// Adding entitlements incrementally — once an entitlement is added to the set,
/// previously-denied features become Allowed.
#[tokio::test]
async fn incremental_entitlement_grants_access() {
    let gate = DefaultFeatureGate::v1_defaults();

    let mut entitlements = EntitlementSet::new(tenant_id(), ProductTier::EnterpriseSelfHosted);

    // No entitlements yet → advanced_admin denied.
    assert!(
        matches!(
            gate.check(&entitlements, "advanced_admin"),
            FeatureGateResult::Denied { .. }
        ),
        "advanced_admin must be Denied before AdvancedAdmin entitlement is added"
    );

    // Grant AdvancedAdmin → advanced_admin now allowed.
    entitlements = entitlements.with_entitlement(Entitlement::AdvancedAdmin);
    assert_eq!(
        gate.check(&entitlements, "advanced_admin"),
        FeatureGateResult::Allowed,
        "advanced_admin must be Allowed after AdvancedAdmin entitlement is granted"
    );

    // GovernanceCompliance features still denied (different entitlement).
    assert!(
        matches!(
            gate.check(&entitlements, "advanced_audit_export"),
            FeatureGateResult::Denied { .. }
        ),
        "advanced_audit_export must still be Denied — AdvancedAdmin != GovernanceCompliance"
    );
}

/// Full pipeline: license from store → build EntitlementSet → gate check.
///
/// Proves that the LicenseRecord coming out of the read-model can be used to
/// drive real access-control decisions through DefaultFeatureGate.
#[tokio::test]
async fn full_pipeline_license_to_gate_decision() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[license_activated_event(ProductTier::EnterpriseSelfHosted)])
        .await
        .unwrap();

    let license = LicenseReadModel::get_active(store.as_ref(), &tenant_id())
        .await
        .unwrap()
        .unwrap();

    // Build entitlement set from license: enterprise tier + GovernanceCompliance.
    let entitlements = EntitlementSet {
        tenant_id: license.tenant_id.clone(),
        tier: license.tier,
        active: vec![Entitlement::GovernanceCompliance, Entitlement::AdvancedAdmin],
    };

    let gate = DefaultFeatureGate::v1_defaults();

    // GA features: always allowed.
    assert_eq!(gate.check(&entitlements, "runtime_core"), FeatureGateResult::Allowed);
    assert_eq!(gate.check(&entitlements, "retrieval_core"), FeatureGateResult::Allowed);

    // Entitlement-gated features: allowed because entitlements are active.
    assert_eq!(gate.check(&entitlements, "advanced_audit_export"), FeatureGateResult::Allowed);
    assert_eq!(gate.check(&entitlements, "advanced_admin"), FeatureGateResult::Allowed);

    // Unknown feature: denied (fail-closed).
    assert!(matches!(
        gate.check(&entitlements, "not_a_real_feature"),
        FeatureGateResult::Denied { .. }
    ));
}
