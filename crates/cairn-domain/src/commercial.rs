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
        ])
    }
}

impl FeatureGate for DefaultFeatureGate {
    fn check(&self, entitlements: &EntitlementSet, feature: &str) -> FeatureGateResult {
        match self.mappings.iter().find(|m| m.feature_name == feature) {
            None => FeatureGateResult::Allowed, // Unknown features default to allowed
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

    #[test]
    fn unknown_feature_defaults_to_allowed() {
        let gate = DefaultFeatureGate::v1_defaults();
        let set = EntitlementSet::new(TenantId::new("t1"), ProductTier::LocalEval);
        assert_eq!(
            gate.check(&set, "nonexistent_feature"),
            FeatureGateResult::Allowed
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
}
