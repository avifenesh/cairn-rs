//! RFC 014 commercial gate enforcement integration tests.
//!
//! Validates the feature gating pipeline:
//! - DefaultFeatureGate returns Allowed for GA features without entitlements.
//! - Entitlement-gated features return Denied when the entitlement is absent.
//! - Unknown features are Denied (fail-closed per RFC 014).
//! - EntitlementOverrideSet events persist and can be read back to unlock gates.
//! - CapabilityMapping correctly links features to required entitlements.

use std::sync::Arc;

use cairn_domain::{
    commercial::{
        CapabilityMapping, DefaultFeatureGate, Entitlement, EntitlementSet, FeatureFlag,
        FeatureGate, FeatureGateResult, ProductTier,
    },
    EntitlementOverrideSet, EventEnvelope, EventId, EventSource, RuntimeEvent, TenantId,
};
use cairn_store::{projections::LicenseReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn tenant_id() -> TenantId {
    TenantId::new("tenant_gate")
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::System, payload.into())
}

fn override_event(feature: &str, allowed: bool) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_override_{feature}"),
        RuntimeEvent::EntitlementOverrideSet(EntitlementOverrideSet {
            tenant_id: tenant_id(),
            feature: feature.to_owned(),
            allowed,
            reason: Some(format!("test override: {feature}={allowed}")),
            set_at_ms: 1_000_000,
        }),
    )
}

/// Build an EntitlementSet from a base tier plus any active overrides.
///
/// This mirrors what the application layer does: reads the override records
/// for a tenant, inspects which features are unlocked (allowed=true), and
/// adds the corresponding entitlements to the set.
///
/// The feature→entitlement mapping is defined by RFC 014 §4:
/// - GovernanceCompliance gates: advanced_audit_export, compliance_policy_packs, approval_hardening
/// - AdvancedAdmin gates: advanced_admin
fn build_entitlement_set_from_overrides(
    tier: ProductTier,
    overrides: &[cairn_domain::commercial::EntitlementOverrideRecord],
) -> EntitlementSet {
    let mut set = EntitlementSet::new(tenant_id(), tier);

    for rec in overrides {
        if !rec.allowed {
            continue; // override denies access — skip
        }
        // Map feature name → required entitlement (per RFC 014 CapabilityMapping).
        let entitlement = match rec.feature.as_str() {
            "advanced_audit_export" | "compliance_policy_packs" | "approval_hardening" => {
                Some(Entitlement::GovernanceCompliance)
            }
            "advanced_admin" => Some(Entitlement::AdvancedAdmin),
            _ => None,
        };
        if let Some(e) = entitlement {
            set = set.with_entitlement(e);
        }
    }

    set
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2) Create DefaultFeatureGate; GA features return Allowed without entitlements.
#[test]
fn ga_features_allowed_without_entitlements() {
    let gate = DefaultFeatureGate::v1_defaults();

    // Bare entitlement set: no tier-specific entitlements.
    let bare = EntitlementSet::new(tenant_id(), ProductTier::TeamSelfHosted);

    assert_eq!(
        gate.check(&bare, "runtime_core"),
        FeatureGateResult::Allowed,
        "runtime_core (GA) must be Allowed without entitlements"
    );
    assert_eq!(
        gate.check(&bare, "retrieval_core"),
        FeatureGateResult::Allowed,
        "retrieval_core (GA) must be Allowed without entitlements"
    );

    // GA features must also pass for LocalEval tier with no entitlements.
    let minimal = EntitlementSet::new(tenant_id(), ProductTier::LocalEval);
    for feature in &["runtime_core", "retrieval_core"] {
        assert_eq!(
            gate.check(&minimal, feature),
            FeatureGateResult::Allowed,
            "GA feature '{feature}' must be Allowed for any tier"
        );
    }
}

/// (3) Gated features return Denied when the required entitlement is absent.
#[test]
fn gated_features_denied_without_entitlement() {
    let gate = DefaultFeatureGate::v1_defaults();
    let bare = EntitlementSet::new(tenant_id(), ProductTier::TeamSelfHosted);

    let gated_features = [
        "advanced_audit_export",
        "compliance_policy_packs",
        "approval_hardening",
        "advanced_admin",
    ];

    for feature in &gated_features {
        let result = gate.check(&bare, feature);
        assert!(
            matches!(result, FeatureGateResult::Denied { .. }),
            "gated feature '{feature}' must return Denied without the required entitlement, \
             got: {result:?}"
        );
    }

    // Wrong entitlement: AdvancedAdmin does not unlock GovernanceCompliance features.
    let wrong_ent = EntitlementSet::new(tenant_id(), ProductTier::EnterpriseSelfHosted)
        .with_entitlement(Entitlement::AdvancedAdmin);

    assert!(
        matches!(
            gate.check(&wrong_ent, "advanced_audit_export"),
            FeatureGateResult::Denied { .. }
        ),
        "advanced_audit_export requires GovernanceCompliance, not AdvancedAdmin"
    );
}

/// (4) Unknown feature names return Denied — RFC 014 fail-closed requirement.
///
/// Defaulting unknown features to Allowed would silently grant access to
/// anything not explicitly listed — the opposite of a secure-by-default gate.
#[test]
fn unknown_features_denied_fail_closed() {
    let gate = DefaultFeatureGate::v1_defaults();
    let enterprise = EntitlementSet::new(tenant_id(), ProductTier::EnterpriseSelfHosted)
        .with_entitlement(Entitlement::GovernanceCompliance)
        .with_entitlement(Entitlement::AdvancedAdmin);

    // Unknown feature must be Denied even when all entitlements are present.
    let unknown_cases = [
        "definitely_unknown_feature",
        "runtime_core_v2", // near-miss: not a registered feature
        "",                // empty string
        "RUNTIME_CORE",    // wrong case (exact match required)
    ];

    for feature in &unknown_cases {
        let result = gate.check(&enterprise, feature);
        assert!(
            matches!(result, FeatureGateResult::Denied { .. }),
            "unknown feature '{feature}' must return Denied (fail-closed), got: {result:?}"
        );
    }
}

/// (5) Appending EntitlementOverrideSet event persists the override,
/// which can then be read and used to unlock a gated feature.
///
/// This is the full RFC 014 commercial gate enforcement pipeline:
/// event → store → read → build set → gate decision.
#[tokio::test]
async fn entitlement_override_event_unlocks_gated_feature() {
    let store = Arc::new(InMemoryStore::new());
    let gate = DefaultFeatureGate::v1_defaults();

    // Before any overrides: gated feature must be Denied.
    let no_overrides = build_entitlement_set_from_overrides(ProductTier::TeamSelfHosted, &[]);
    assert!(
        matches!(
            gate.check(&no_overrides, "advanced_audit_export"),
            FeatureGateResult::Denied { .. }
        ),
        "advanced_audit_export must be Denied before override"
    );

    // Append an EntitlementOverrideSet that grants advanced_audit_export.
    store
        .append(&[override_event("advanced_audit_export", true)])
        .await
        .unwrap();

    // Read overrides from the store.
    let overrides = LicenseReadModel::list_overrides(store.as_ref(), &tenant_id())
        .await
        .unwrap();
    assert_eq!(overrides.len(), 1, "one override must be stored");
    assert_eq!(overrides[0].feature, "advanced_audit_export");
    assert!(overrides[0].allowed, "override must be granted");

    // Build EntitlementSet from the persisted overrides.
    let with_override =
        build_entitlement_set_from_overrides(ProductTier::TeamSelfHosted, &overrides);

    // Gate must now allow the feature.
    assert_eq!(
        gate.check(&with_override, "advanced_audit_export"),
        FeatureGateResult::Allowed,
        "advanced_audit_export must be Allowed after override is persisted and applied"
    );

    // Other gated features NOT in the override must still be Denied.
    assert!(
        matches!(
            gate.check(&with_override, "advanced_admin"),
            FeatureGateResult::Denied { .. }
        ),
        "advanced_admin must remain Denied — no override was set for it"
    );

    // GA features still pass regardless.
    assert_eq!(
        gate.check(&with_override, "runtime_core"),
        FeatureGateResult::Allowed,
        "GA feature must remain Allowed"
    );
}

/// Multiple overrides: granting and denying independently.
#[tokio::test]
async fn multiple_overrides_grant_and_deny_independently() {
    let store = Arc::new(InMemoryStore::new());
    let gate = DefaultFeatureGate::v1_defaults();

    // Grant advanced_audit_export, deny advanced_admin.
    store
        .append(&[
            override_event("advanced_audit_export", true),
            override_event("advanced_admin", false),
        ])
        .await
        .unwrap();

    let overrides = LicenseReadModel::list_overrides(store.as_ref(), &tenant_id())
        .await
        .unwrap();
    assert_eq!(overrides.len(), 2);

    let set = build_entitlement_set_from_overrides(ProductTier::EnterpriseSelfHosted, &overrides);

    assert_eq!(
        gate.check(&set, "advanced_audit_export"),
        FeatureGateResult::Allowed,
        "granted override must unlock advanced_audit_export"
    );
    assert!(
        matches!(
            gate.check(&set, "advanced_admin"),
            FeatureGateResult::Denied { .. }
        ),
        "denied override means advanced_admin stays locked"
    );
}

/// (6) CapabilityMapping correctly links features to their required entitlements.
///
/// Tests the structure of the default mappings by verifying that:
/// - Every EntitlementGated feature with a required_entitlement is actually gated.
/// - Features with flag=GA are always allowed regardless of entitlements.
/// - Features with flag=Preview are also always allowed.
#[test]
fn capability_mapping_links_features_to_entitlements() {
    let gate = DefaultFeatureGate::v1_defaults();
    let bare = EntitlementSet::new(tenant_id(), ProductTier::TeamSelfHosted);
    let full = EntitlementSet::new(tenant_id(), ProductTier::EnterpriseSelfHosted)
        .with_entitlement(Entitlement::GovernanceCompliance)
        .with_entitlement(Entitlement::AdvancedAdmin)
        .with_entitlement(Entitlement::ManagedServiceRights);

    // Define known mappings and expected behaviour.
    let mappings: &[(&str, FeatureFlag, Option<Entitlement>)] = &[
        ("runtime_core", FeatureFlag::GeneralAvailability, None),
        ("retrieval_core", FeatureFlag::GeneralAvailability, None),
        (
            "advanced_audit_export",
            FeatureFlag::EntitlementGated,
            Some(Entitlement::GovernanceCompliance),
        ),
        (
            "compliance_policy_packs",
            FeatureFlag::EntitlementGated,
            Some(Entitlement::GovernanceCompliance),
        ),
        (
            "approval_hardening",
            FeatureFlag::EntitlementGated,
            Some(Entitlement::GovernanceCompliance),
        ),
        (
            "advanced_admin",
            FeatureFlag::EntitlementGated,
            Some(Entitlement::AdvancedAdmin),
        ),
    ];

    for (feature, flag, required) in mappings {
        match flag {
            FeatureFlag::GeneralAvailability | FeatureFlag::Preview => {
                // GA / Preview features must always be Allowed regardless of entitlements.
                assert_eq!(
                    gate.check(&bare, feature),
                    FeatureGateResult::Allowed,
                    "{flag:?} feature '{feature}' must be Allowed without entitlements"
                );
                assert_eq!(
                    gate.check(&full, feature),
                    FeatureGateResult::Allowed,
                    "{flag:?} feature '{feature}' must be Allowed with full entitlements"
                );
            }
            FeatureFlag::EntitlementGated => {
                // Gated features must be Denied without the required entitlement.
                assert!(
                    matches!(gate.check(&bare, feature), FeatureGateResult::Denied { .. }),
                    "EntitlementGated feature '{feature}' must be Denied without entitlements"
                );

                // With the correct entitlement, must be Allowed.
                if let Some(required_ent) = required {
                    let set_with_ent =
                        EntitlementSet::new(tenant_id(), ProductTier::EnterpriseSelfHosted)
                            .with_entitlement(*required_ent);
                    assert_eq!(
                        gate.check(&set_with_ent, feature),
                        FeatureGateResult::Allowed,
                        "feature '{feature}' must be Allowed when {:?} entitlement is present",
                        required_ent
                    );
                }
            }
        }
    }
}

/// Custom CapabilityMapping: a bespoke gate can define its own feature registry.
#[test]
fn custom_capability_mapping_works_as_gate() {
    let gate = DefaultFeatureGate::new(vec![
        CapabilityMapping {
            feature_name: "custom_feature_a".to_owned(),
            required_entitlement: None,
            flag: FeatureFlag::GeneralAvailability,
        },
        CapabilityMapping {
            feature_name: "custom_feature_b".to_owned(),
            required_entitlement: Some(Entitlement::ManagedServiceRights),
            flag: FeatureFlag::EntitlementGated,
        },
    ]);

    let bare = EntitlementSet::new(tenant_id(), ProductTier::TeamSelfHosted);
    let with_managed = EntitlementSet::new(tenant_id(), ProductTier::EnterpriseSelfHosted)
        .with_entitlement(Entitlement::ManagedServiceRights);

    // GA custom feature always allowed.
    assert_eq!(
        gate.check(&bare, "custom_feature_a"),
        FeatureGateResult::Allowed
    );

    // Gated custom feature denied without entitlement.
    assert!(matches!(
        gate.check(&bare, "custom_feature_b"),
        FeatureGateResult::Denied { .. }
    ));

    // Gated custom feature allowed with correct entitlement.
    assert_eq!(
        gate.check(&with_managed, "custom_feature_b"),
        FeatureGateResult::Allowed
    );

    // Features from v1_defaults must be unknown to this custom gate (fail-closed).
    assert!(
        matches!(
            gate.check(&bare, "runtime_core"),
            FeatureGateResult::Denied { .. }
        ),
        "features not in custom registry must be denied"
    );
}
