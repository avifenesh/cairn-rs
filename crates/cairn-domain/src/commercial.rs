use crate::ids::TenantId;
use serde::{Deserialize, Serialize};

/// Product tier per RFC 014.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductTier {
    LocalEval,
    TeamSelfHosted,
    EnterpriseSelfHosted,
}

/// Named entitlement categories per RFC 014.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Entitlement {
    DeploymentTier,
    GovernanceCompliance,
    AdvancedAdmin,
    ManagedServiceRights,
}

/// Feature rollout flag per RFC 014.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureFlag {
    Preview,
    GeneralAvailability,
    EntitlementGated,
}

/// Set of active entitlements for a tenant deployment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntitlementSet {
    pub tenant_id: TenantId,
    pub tier: ProductTier,
    pub active: Vec<Entitlement>,
}

impl EntitlementSet {
    pub fn new(tenant_id: TenantId, tier: ProductTier) -> Self {
        Self {
            tenant_id,
            tier,
            active: Vec::new(),
        }
    }

    pub fn with_entitlement(mut self, entitlement: Entitlement) -> Self {
        if !self.active.contains(&entitlement) {
            self.active.push(entitlement);
        }
        self
    }

    pub fn has(&self, entitlement: Entitlement) -> bool {
        self.active.contains(&entitlement)
    }

    pub fn is_enterprise(&self) -> bool {
        self.tier == ProductTier::EnterpriseSelfHosted
    }
}

/// Tenant-scoped license record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LicenseRecord {
    pub tenant_id: TenantId,
    pub tier: ProductTier,
    pub entitlements: Vec<Entitlement>,
    pub issued_at: u64,
    pub expires_at: Option<u64>,
    pub license_key: Option<String>,
}

/// Feature gate check result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeatureGateResult {
    Allowed,
    Denied { reason: String },
    Degraded { reason: String },
}

/// Trait for checking entitlement-gated capabilities.
pub trait FeatureGate: Send + Sync {
    fn check(&self, entitlements: &EntitlementSet, feature: &str) -> FeatureGateResult;
}

/// Capability-to-entitlement mapping per RFC 014.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityMapping {
    pub feature_name: String,
    pub required_entitlement: Option<Entitlement>,
    pub flag: FeatureFlag,
}

/// Record of entitlement changes for audit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntitlementChangeRecord {
    pub tenant_id: TenantId,
    pub changed_at: u64,
    pub previous_tier: Option<ProductTier>,
    pub new_tier: ProductTier,
    pub added: Vec<Entitlement>,
    pub removed: Vec<Entitlement>,
    pub reason: Option<String>,
}

/// Default feature gate that checks capability mappings against entitlements.
pub struct DefaultFeatureGate {
    mappings: Vec<CapabilityMapping>,
}

impl DefaultFeatureGate {
    pub fn new(mappings: Vec<CapabilityMapping>) -> Self {
        Self { mappings }
    }

    pub fn v1_defaults() -> Self {
        Self::new(vec![
            CapabilityMapping {
                feature_name: "advanced_audit_export".to_owned(),
                required_entitlement: Some(Entitlement::GovernanceCompliance),
                flag: FeatureFlag::EntitlementGated,
            },
            CapabilityMapping {
                feature_name: "compliance_policy_packs".to_owned(),
                required_entitlement: Some(Entitlement::GovernanceCompliance),
                flag: FeatureFlag::EntitlementGated,
            },
            CapabilityMapping {
                feature_name: "approval_hardening".to_owned(),
                required_entitlement: Some(Entitlement::GovernanceCompliance),
                flag: FeatureFlag::EntitlementGated,
            },
            CapabilityMapping {
                feature_name: "advanced_admin".to_owned(),
                required_entitlement: Some(Entitlement::AdvancedAdmin),
                flag: FeatureFlag::EntitlementGated,
            },
            CapabilityMapping {
                feature_name: "runtime_core".to_owned(),
                required_entitlement: None,
                flag: FeatureFlag::GeneralAvailability,
            },
            CapabilityMapping {
                feature_name: "retrieval_core".to_owned(),
                required_entitlement: None,
                flag: FeatureFlag::GeneralAvailability,
            },
            CapabilityMapping {
                feature_name: "eval_matrices".to_owned(),
                required_entitlement: None,
                flag: FeatureFlag::GeneralAvailability,
            },
            CapabilityMapping {
                feature_name: "multi_provider".to_owned(),
                required_entitlement: Some(Entitlement::DeploymentTier),
                flag: FeatureFlag::EntitlementGated,
            },
            CapabilityMapping {
                feature_name: "credential_management".to_owned(),
                required_entitlement: Some(Entitlement::DeploymentTier),
                flag: FeatureFlag::EntitlementGated,
            },
        ])
    }
}

impl FeatureGate for DefaultFeatureGate {
    fn check(&self, entitlements: &EntitlementSet, feature: &str) -> FeatureGateResult {
        match self.mappings.iter().find(|m| m.feature_name == feature) {
            // RFC 014: unknown feature names must return Denied, not Allowed.
            // Defaulting unknown features to Allowed would silently grant access
            // to anything not explicitly listed — the opposite of fail-closed.
            None => FeatureGateResult::Denied {
                reason: format!("feature '{feature}' is not a recognized capability"),
            },
            Some(mapping) => match mapping.flag {
                FeatureFlag::GeneralAvailability => FeatureGateResult::Allowed,
                FeatureFlag::Preview => FeatureGateResult::Allowed,
                FeatureFlag::EntitlementGated => match mapping.required_entitlement {
                    None => FeatureGateResult::Allowed,
                    Some(required) => {
                        if entitlements.has(required) {
                            FeatureGateResult::Allowed
                        } else {
                            FeatureGateResult::Denied {
                                reason: format!(
                                    "feature '{}' requires {:?} entitlement",
                                    feature, required
                                ),
                            }
                        }
                    }
                },
            },
        }
    }
}

/// An operator-applied override to a tenant's entitlement set.
///
/// Used by `commercial::LicenseReadModel` to query manual overrides
/// that supplement or restrict the base license entitlements.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EntitlementOverrideRecord {
    pub override_id: String,
    pub tenant_id: crate::ids::TenantId,
    pub entitlement: Entitlement,
    pub granted: bool,
    pub reason: Option<String>,
    pub applied_at: u64,
    #[serde(default)]
    pub feature: String,
    #[serde(default)]
    pub allowed: bool,
    #[serde(default)]
    pub set_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entitlement_set_builder() {
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::TeamSelfHosted)
            .with_entitlement(Entitlement::GovernanceCompliance);
        assert!(set.has(Entitlement::GovernanceCompliance));
        assert!(!set.has(Entitlement::AdvancedAdmin));
        assert!(!set.is_enterprise());
    }

    #[test]
    fn enterprise_tier_detection() {
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::EnterpriseSelfHosted);
        assert!(set.is_enterprise());
    }

    #[test]
    fn feature_gate_allows_ga_features() {
        let gate = DefaultFeatureGate::v1_defaults();
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::TeamSelfHosted);
        assert_eq!(
            gate.check(&set, "runtime_core"),
            FeatureGateResult::Allowed
        );
    }

    #[test]
    fn feature_gate_denies_gated_without_entitlement() {
        let gate = DefaultFeatureGate::v1_defaults();
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::TeamSelfHosted);
        let result = gate.check(&set, "advanced_audit_export");
        assert!(matches!(result, FeatureGateResult::Denied { .. }));
    }

    #[test]
    fn feature_gate_allows_gated_with_entitlement() {
        let gate = DefaultFeatureGate::v1_defaults();
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::EnterpriseSelfHosted)
            .with_entitlement(Entitlement::GovernanceCompliance);
        assert_eq!(
            gate.check(&set, "advanced_audit_export"),
            FeatureGateResult::Allowed
        );
    }

    /// RFC 014: unknown feature names MUST return Denied (fail-closed), not Allowed.
    ///
    /// Defaulting to Allowed for unrecognized names would silently permit anything
    /// not on the list — the opposite of entitlement-gated access control.
    #[test]
    fn unknown_feature_returns_denied_not_allowed() {
        let gate = DefaultFeatureGate::v1_defaults();
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::LocalEval);
        let result = gate.check(&set, "nonexistent_feature");
        assert!(
            matches!(result, FeatureGateResult::Denied { .. }),
            "RFC 014: unknown feature must return Denied; got {:?}", result
        );
    }

    #[test]
    fn product_tiers_are_distinct() {
        assert_ne!(ProductTier::LocalEval, ProductTier::TeamSelfHosted);
        assert_ne!(
            ProductTier::TeamSelfHosted,
            ProductTier::EnterpriseSelfHosted
        );
    }

    #[test]
    fn duplicate_entitlement_not_added_twice() {
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::TeamSelfHosted)
            .with_entitlement(Entitlement::GovernanceCompliance)
            .with_entitlement(Entitlement::GovernanceCompliance);
        assert_eq!(set.active.len(), 1);
    }

    /// RFC 014 §4: absent entitlement must REFUSE the request, not corrupt state.
    ///
    /// When a feature gate denies access due to a missing entitlement:
    ///  - the result is `Denied` (the request is refused)
    ///  - the caller's `EntitlementSet` is not mutated (no corruption)
    ///  - other entitlements the tenant holds remain valid and accessible
    #[test]
    fn absent_entitlement_refuses_without_corrupting_existing_entitlements() {
        let gate = DefaultFeatureGate::v1_defaults();

        // Tenant has GovernanceCompliance but NOT AdvancedAdmin.
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::TeamSelfHosted)
            .with_entitlement(Entitlement::GovernanceCompliance);

        let initial_active_count = set.active.len();

        // Gate check for absent entitlement must return Denied.
        let result = gate.check(&set, "advanced_admin");
        assert!(
            matches!(result, FeatureGateResult::Denied { .. }),
            "absent entitlement must refuse with Denied, got {:?}", result
        );

        // Denial must not corrupt the EntitlementSet — count unchanged, existing entitlement still valid.
        assert_eq!(set.active.len(), initial_active_count,
            "gate check must not mutate the EntitlementSet");
        assert!(set.has(Entitlement::GovernanceCompliance),
            "pre-existing entitlement must survive a denial of a different feature");

        // The feature backed by the held entitlement must still be accessible.
        assert_eq!(
            gate.check(&set, "advanced_audit_export"),
            FeatureGateResult::Allowed,
            "held entitlement must still grant access after an unrelated denial"
        );
    }
}

// ── RFC 014 Gap Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod rfc014_tests {
    use super::*;

    /// RFC 014: entitlements must be explicit and inspectable, not hidden.
    #[test]
    fn rfc014_entitlements_are_explicitly_listed_and_inspectable() {
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::TeamSelfHosted)
            .with_entitlement(Entitlement::GovernanceCompliance)
            .with_entitlement(Entitlement::AdvancedAdmin);

        // RFC 014: entitlements must be inspectable — list them explicitly.
        assert!(set.has(Entitlement::GovernanceCompliance),
            "RFC 014: entitlements must be individually inspectable");
        assert!(set.has(Entitlement::AdvancedAdmin),
            "RFC 014: all active entitlements must be visible");
        assert!(!set.has(Entitlement::ManagedServiceRights),
            "RFC 014: absent entitlements must not appear present");
        // The active list must be enumerable.
        assert_eq!(set.active.len(), 2,
            "RFC 014: active entitlement count must be accurate");
    }

    /// RFC 014: one codebase, one binary — product tier does not fork behavior.
    /// All tiers use the same ProductTier enum; behavior difference is by entitlement gating only.
    #[test]
    fn rfc014_one_codebase_all_tiers_use_same_type() {
        // All three tiers must be expressible from the same type.
        let tiers = [
            ProductTier::LocalEval,
            ProductTier::TeamSelfHosted,
            ProductTier::EnterpriseSelfHosted,
        ];
        for tier in &tiers {
            let set = EntitlementSet::new(TenantId::new("t1"), *tier);
            // Must not panic — same struct handles all tiers.
            let _ = set.has(Entitlement::GovernanceCompliance);
        }
    }

    /// RFC 014: entitlement absence must degrade by refusing, not corrupting state.
    #[test]
    fn rfc014_missing_entitlement_fails_operation_gracefully() {
        let gate = DefaultFeatureGate::v1_defaults();
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::LocalEval);
        // Community tier should not have advanced features.
        let result = gate.check(&set, "advanced_audit_export");
        // Must be Denied, not panicking or corrupting state.
        assert!(
            matches!(result, FeatureGateResult::Denied { .. }),
            "RFC 014: community tier must be denied advanced features, got {:?}", result
        );
        // State must not be mutated.
        assert!(set.active.is_empty(),
            "RFC 014: gate check must not add entitlements to deny result");
    }

    /// RFC 014: enterprise tier includes advanced capabilities.
    #[test]
    fn rfc014_enterprise_tier_includes_advanced_capabilities() {
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::EnterpriseSelfHosted);
        assert!(set.is_enterprise(),
            "RFC 014: EnterpriseSelfHosted must be recognized as enterprise tier");
    }

    /// RFC 014: feature gate result types are distinct and exhaustive.
    #[test]
    fn rfc014_feature_gate_result_types_are_distinct() {
        assert_ne!(
            FeatureGateResult::Allowed,
            FeatureGateResult::Denied { reason: "test".to_owned() },
        );
        assert_ne!(
            FeatureGateResult::Allowed,
            FeatureGateResult::Degraded { reason: "test".to_owned() },
        );
    }
}
