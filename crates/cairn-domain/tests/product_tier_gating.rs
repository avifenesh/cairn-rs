//! RFC 014 product tier gating tests.
//!
//! Validates the commercial gating model:
//! - ProductTier, Entitlement, FeatureFlag serde round-trips.
//! - EntitlementSet.has() and with_entitlement() semantics.
//! - DefaultFeatureGate behaviour across all three product tiers.
//! - FeatureFlag semantics: Preview, GA, EntitlementGated.
//! - CapabilityMapping links each named feature to its required entitlement.

use cairn_domain::{
    commercial::{
        CapabilityMapping, DefaultFeatureGate, Entitlement, EntitlementSet, FeatureFlag,
        FeatureGate, FeatureGateResult, ProductTier,
    },
    TenantId,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn tenant() -> TenantId {
    TenantId::new("tenant_tier")
}

fn local() -> EntitlementSet {
    EntitlementSet::new(tenant(), ProductTier::LocalEval)
}
fn team() -> EntitlementSet {
    EntitlementSet::new(tenant(), ProductTier::TeamSelfHosted)
}
fn enterprise() -> EntitlementSet {
    EntitlementSet::new(tenant(), ProductTier::EnterpriseSelfHosted)
}

// ── (1): ProductTier serde round-trip ─────────────────────────────────────────

/// All three ProductTier variants serde round-trip correctly.
#[test]
fn product_tier_serde_round_trip() {
    for tier in [
        ProductTier::LocalEval,
        ProductTier::TeamSelfHosted,
        ProductTier::EnterpriseSelfHosted,
    ] {
        let json = serde_json::to_string(&tier).unwrap();
        let back: ProductTier = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tier, "serde round-trip must be identity for {tier:?}");
    }
}

/// ProductTier serialises to expected snake_case strings.
#[test]
fn product_tier_serializes_to_snake_case() {
    assert_eq!(
        serde_json::to_string(&ProductTier::LocalEval).unwrap(),
        r#""local_eval""#
    );
    assert_eq!(
        serde_json::to_string(&ProductTier::TeamSelfHosted).unwrap(),
        r#""team_self_hosted""#
    );
    assert_eq!(
        serde_json::to_string(&ProductTier::EnterpriseSelfHosted).unwrap(),
        r#""enterprise_self_hosted""#
    );
}

/// All three tiers are distinct.
#[test]
fn product_tier_variants_are_distinct() {
    assert_ne!(ProductTier::LocalEval, ProductTier::TeamSelfHosted);
    assert_ne!(
        ProductTier::TeamSelfHosted,
        ProductTier::EnterpriseSelfHosted
    );
    assert_ne!(ProductTier::LocalEval, ProductTier::EnterpriseSelfHosted);
}

/// Entitlement and FeatureFlag variants serde correctly.
#[test]
fn entitlement_and_feature_flag_serde_round_trips() {
    for ent in [
        Entitlement::DeploymentTier,
        Entitlement::GovernanceCompliance,
        Entitlement::AdvancedAdmin,
        Entitlement::ManagedServiceRights,
    ] {
        let json = serde_json::to_string(&ent).unwrap();
        let back: Entitlement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ent);
    }

    for flag in [
        FeatureFlag::Preview,
        FeatureFlag::GeneralAvailability,
        FeatureFlag::EntitlementGated,
    ] {
        let json = serde_json::to_string(&flag).unwrap();
        let back: FeatureFlag = serde_json::from_str(&json).unwrap();
        assert_eq!(back, flag);
    }
}

// ── (2): EntitlementSet.has() ──────────────────────────────────────────────────

/// has() returns false when the entitlement is not in the active set.
#[test]
fn entitlement_set_has_returns_false_when_absent() {
    let set = local();
    assert!(!set.has(Entitlement::GovernanceCompliance));
    assert!(!set.has(Entitlement::AdvancedAdmin));
    assert!(!set.has(Entitlement::ManagedServiceRights));
    assert!(!set.has(Entitlement::DeploymentTier));
}

/// has() returns true only for the entitlements explicitly granted.
#[test]
fn entitlement_set_has_returns_true_when_present() {
    let set = team().with_entitlement(Entitlement::GovernanceCompliance);
    assert!(set.has(Entitlement::GovernanceCompliance));
    // Other entitlements are still absent.
    assert!(!set.has(Entitlement::AdvancedAdmin));
    assert!(!set.has(Entitlement::ManagedServiceRights));
}

/// is_enterprise() returns true only for EnterpriseSelfHosted tier.
#[test]
fn is_enterprise_only_true_for_enterprise_tier() {
    assert!(!local().is_enterprise(), "LocalEval is not enterprise");
    assert!(!team().is_enterprise(), "TeamSelfHosted is not enterprise");
    assert!(
        enterprise().is_enterprise(),
        "EnterpriseSelfHosted must be enterprise"
    );
}

/// has() checks across all four entitlement categories independently.
#[test]
fn entitlement_set_has_checks_each_category_independently() {
    let set = enterprise()
        .with_entitlement(Entitlement::GovernanceCompliance)
        .with_entitlement(Entitlement::AdvancedAdmin);

    assert!(set.has(Entitlement::GovernanceCompliance));
    assert!(set.has(Entitlement::AdvancedAdmin));
    // ManagedServiceRights and DeploymentTier not granted.
    assert!(!set.has(Entitlement::ManagedServiceRights));
    assert!(!set.has(Entitlement::DeploymentTier));
}

// ── (3): with_entitlement() immutability ──────────────────────────────────────

/// with_entitlement() is builder-style: the original set is consumed, a new
/// set is returned. Calling it multiple times accumulates entitlements.
#[test]
fn with_entitlement_is_builder_style_accumulation() {
    let base = local();
    assert!(!base.has(Entitlement::GovernanceCompliance));

    // with_entitlement returns a new set with the entitlement added.
    let extended = base.with_entitlement(Entitlement::GovernanceCompliance);
    assert!(extended.has(Entitlement::GovernanceCompliance));

    // Chain: each call adds one more entitlement.
    let full = enterprise()
        .with_entitlement(Entitlement::GovernanceCompliance)
        .with_entitlement(Entitlement::AdvancedAdmin)
        .with_entitlement(Entitlement::ManagedServiceRights)
        .with_entitlement(Entitlement::DeploymentTier);

    for ent in [
        Entitlement::GovernanceCompliance,
        Entitlement::AdvancedAdmin,
        Entitlement::ManagedServiceRights,
        Entitlement::DeploymentTier,
    ] {
        assert!(full.has(ent), "chained with_entitlement must add {ent:?}");
    }
    assert_eq!(
        full.active.len(),
        4,
        "four distinct entitlements must be present"
    );
}

/// Duplicate entitlements are deduplicated — with_entitlement is idempotent.
#[test]
fn with_entitlement_deduplicates() {
    let set = enterprise()
        .with_entitlement(Entitlement::GovernanceCompliance)
        .with_entitlement(Entitlement::GovernanceCompliance)
        .with_entitlement(Entitlement::GovernanceCompliance);

    assert_eq!(
        set.active.len(),
        1,
        "duplicate entitlements must be deduplicated"
    );
    assert!(set.has(Entitlement::GovernanceCompliance));
}

// ── (4): DefaultFeatureGate with all 3 tiers ──────────────────────────────────

/// LocalEval tier with no entitlements: GA features allowed, gated features denied.
#[test]
fn local_eval_tier_ga_allowed_gated_denied() {
    let gate = DefaultFeatureGate::v1_defaults();
    let set = local();

    assert_eq!(gate.check(&set, "runtime_core"), FeatureGateResult::Allowed);
    assert_eq!(
        gate.check(&set, "retrieval_core"),
        FeatureGateResult::Allowed
    );

    assert!(matches!(
        gate.check(&set, "advanced_audit_export"),
        FeatureGateResult::Denied { .. }
    ));
    assert!(matches!(
        gate.check(&set, "advanced_admin"),
        FeatureGateResult::Denied { .. }
    ));
}

/// TeamSelfHosted tier with GovernanceCompliance: unlocks governance features.
#[test]
fn team_tier_with_governance_unlocks_compliance_features() {
    let gate = DefaultFeatureGate::v1_defaults();
    let set = team().with_entitlement(Entitlement::GovernanceCompliance);

    assert_eq!(
        gate.check(&set, "advanced_audit_export"),
        FeatureGateResult::Allowed
    );
    assert_eq!(
        gate.check(&set, "compliance_policy_packs"),
        FeatureGateResult::Allowed
    );
    assert_eq!(
        gate.check(&set, "approval_hardening"),
        FeatureGateResult::Allowed
    );

    // advanced_admin still denied (requires AdvancedAdmin entitlement, not GovernanceCompliance).
    assert!(matches!(
        gate.check(&set, "advanced_admin"),
        FeatureGateResult::Denied { .. }
    ));

    // GA features always pass.
    assert_eq!(gate.check(&set, "runtime_core"), FeatureGateResult::Allowed);
}

/// EnterpriseSelfHosted with full entitlements: all features allowed.
#[test]
fn enterprise_tier_with_all_entitlements_unlocks_everything() {
    let gate = DefaultFeatureGate::v1_defaults();
    let set = enterprise()
        .with_entitlement(Entitlement::GovernanceCompliance)
        .with_entitlement(Entitlement::AdvancedAdmin);

    for feature in &[
        "runtime_core",
        "retrieval_core",
        "advanced_audit_export",
        "compliance_policy_packs",
        "approval_hardening",
        "advanced_admin",
    ] {
        assert_eq!(
            gate.check(&set, feature),
            FeatureGateResult::Allowed,
            "enterprise tier with all entitlements must allow '{feature}'"
        );
    }
}

/// All three tiers: GA features pass unconditionally.
#[test]
fn ga_features_pass_for_every_tier() {
    let gate = DefaultFeatureGate::v1_defaults();
    for set in [local(), team(), enterprise()] {
        for feature in &["runtime_core", "retrieval_core"] {
            assert_eq!(
                gate.check(&set, feature),
                FeatureGateResult::Allowed,
                "GA feature '{feature}' must pass for tier {:?}",
                set.tier
            );
        }
    }
}

// ── (5): FeatureFlag semantics ─────────────────────────────────────────────────

/// GeneralAvailability: always Allowed regardless of entitlements or tier.
#[test]
fn feature_flag_ga_always_allowed() {
    let gate = DefaultFeatureGate::new(vec![CapabilityMapping {
        feature_name: "test_ga".to_owned(),
        required_entitlement: None,
        flag: FeatureFlag::GeneralAvailability,
    }]);

    for set in [local(), team(), enterprise()] {
        assert_eq!(
            gate.check(&set, "test_ga"),
            FeatureGateResult::Allowed,
            "GA feature must be Allowed for tier {:?}",
            set.tier
        );
    }
}

/// Preview: also always Allowed (preview features are accessible to all tenants).
#[test]
fn feature_flag_preview_always_allowed() {
    let gate = DefaultFeatureGate::new(vec![CapabilityMapping {
        feature_name: "test_preview".to_owned(),
        required_entitlement: None,
        flag: FeatureFlag::Preview,
    }]);

    for set in [local(), team(), enterprise()] {
        assert_eq!(
            gate.check(&set, "test_preview"),
            FeatureGateResult::Allowed,
            "Preview feature must be Allowed for tier {:?}",
            set.tier
        );
    }
}

/// EntitlementGated: Denied without the required entitlement, Allowed with it.
#[test]
fn feature_flag_entitlement_gated_denied_and_allowed() {
    let gate = DefaultFeatureGate::new(vec![CapabilityMapping {
        feature_name: "test_gated".to_owned(),
        required_entitlement: Some(Entitlement::ManagedServiceRights),
        flag: FeatureFlag::EntitlementGated,
    }]);

    let without = enterprise();
    assert!(
        matches!(
            gate.check(&without, "test_gated"),
            FeatureGateResult::Denied { .. }
        ),
        "EntitlementGated feature must be Denied without the required entitlement"
    );

    let with_ent = enterprise().with_entitlement(Entitlement::ManagedServiceRights);
    assert_eq!(
        gate.check(&with_ent, "test_gated"),
        FeatureGateResult::Allowed,
        "EntitlementGated feature must be Allowed when the required entitlement is present"
    );
}

/// EntitlementGated with None required_entitlement is always Allowed (open gate).
#[test]
fn entitlement_gated_with_no_required_entitlement_is_always_allowed() {
    let gate = DefaultFeatureGate::new(vec![CapabilityMapping {
        feature_name: "test_open_gate".to_owned(),
        required_entitlement: None,
        flag: FeatureFlag::EntitlementGated,
    }]);

    // No required entitlement → always allowed even without any active entitlements.
    assert_eq!(
        gate.check(&local(), "test_open_gate"),
        FeatureGateResult::Allowed
    );
}

/// Fail-closed: unknown feature names always return Denied.
#[test]
fn unknown_feature_denied_fail_closed() {
    let gate = DefaultFeatureGate::v1_defaults();
    let full = enterprise()
        .with_entitlement(Entitlement::GovernanceCompliance)
        .with_entitlement(Entitlement::AdvancedAdmin);

    for unknown in &["", "does_not_exist", "runtime_core_v2", "RUNTIME_CORE"] {
        assert!(
            matches!(gate.check(&full, unknown), FeatureGateResult::Denied { .. }),
            "unknown feature '{unknown}' must be Denied (fail-closed)"
        );
    }
}

// ── (6): CapabilityMapping for all named entitlement categories ──────────────

/// CapabilityMapping covers all four Entitlement categories in v1_defaults.
#[test]
fn capability_mapping_covers_all_entitlement_categories() {
    let gate = DefaultFeatureGate::v1_defaults();

    // GovernanceCompliance gates three features.
    for feature in &[
        "advanced_audit_export",
        "compliance_policy_packs",
        "approval_hardening",
    ] {
        let bare = enterprise();
        assert!(
            matches!(gate.check(&bare, feature), FeatureGateResult::Denied { .. }),
            "'{feature}' must require GovernanceCompliance"
        );
        let with_gov = enterprise().with_entitlement(Entitlement::GovernanceCompliance);
        assert_eq!(
            gate.check(&with_gov, feature),
            FeatureGateResult::Allowed,
            "'{feature}' must be unlocked by GovernanceCompliance"
        );
    }

    // AdvancedAdmin gates one feature.
    let bare = enterprise();
    assert!(matches!(
        gate.check(&bare, "advanced_admin"),
        FeatureGateResult::Denied { .. }
    ));
    let with_adm = enterprise().with_entitlement(Entitlement::AdvancedAdmin);
    assert_eq!(
        gate.check(&with_adm, "advanced_admin"),
        FeatureGateResult::Allowed
    );
}

/// CapabilityMapping serde: feature_name, required_entitlement, flag all round-trip.
#[test]
fn capability_mapping_serde_round_trip() {
    let mapping = CapabilityMapping {
        feature_name: "advanced_audit_export".to_owned(),
        required_entitlement: Some(Entitlement::GovernanceCompliance),
        flag: FeatureFlag::EntitlementGated,
    };

    let json = serde_json::to_string(&mapping).unwrap();
    let back: CapabilityMapping = serde_json::from_str(&json).unwrap();

    assert_eq!(back.feature_name, "advanced_audit_export");
    assert_eq!(
        back.required_entitlement,
        Some(Entitlement::GovernanceCompliance)
    );
    assert_eq!(back.flag, FeatureFlag::EntitlementGated);
}

/// Cross-entitlement isolation: granting one entitlement does not grant others.
#[test]
fn entitlements_are_independently_isolated() {
    let gate = DefaultFeatureGate::v1_defaults();

    // Only GovernanceCompliance granted.
    let gov_only = enterprise().with_entitlement(Entitlement::GovernanceCompliance);
    assert_eq!(
        gate.check(&gov_only, "advanced_audit_export"),
        FeatureGateResult::Allowed
    );
    assert!(
        matches!(
            gate.check(&gov_only, "advanced_admin"),
            FeatureGateResult::Denied { .. }
        ),
        "AdvancedAdmin gate must not be unlocked by GovernanceCompliance entitlement"
    );

    // Only AdvancedAdmin granted.
    let adm_only = enterprise().with_entitlement(Entitlement::AdvancedAdmin);
    assert_eq!(
        gate.check(&adm_only, "advanced_admin"),
        FeatureGateResult::Allowed
    );
    assert!(
        matches!(
            gate.check(&adm_only, "advanced_audit_export"),
            FeatureGateResult::Denied { .. }
        ),
        "GovernanceCompliance gate must not be unlocked by AdvancedAdmin entitlement"
    );
}
