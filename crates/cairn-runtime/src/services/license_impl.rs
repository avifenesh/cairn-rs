use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{
    DefaultFeatureGate, EntitlementOverrideRecord, EntitlementOverrideSet, EntitlementSet,
    FeatureGate, FeatureGateResult, LicenseActivated, LicenseRecord, ProductTier, RuntimeEvent,
    TenantId,
};
use cairn_store::projections::LicenseReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::licenses::LicenseService;

pub struct LicenseServiceImpl<S> {
    store: Arc<S>,
    feature_gate: DefaultFeatureGate,
}

impl<S> LicenseServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            feature_gate: DefaultFeatureGate::v1_defaults(),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn next_license_id(tenant_id: &TenantId, issued_at: u64) -> String {
    format!("lic_{}_{}", tenant_id.as_str(), issued_at)
}

#[async_trait]
impl<S> LicenseService for LicenseServiceImpl<S>
where
    S: EventLog + LicenseReadModel + 'static,
{
    async fn activate(
        &self,
        tenant_id: TenantId,
        tier: ProductTier,
        valid_until_ms: Option<u64>,
    ) -> Result<LicenseRecord, RuntimeError> {
        let issued_at = now_ms();
        let license_id = next_license_id(&tenant_id, issued_at);
        let event = make_envelope(RuntimeEvent::LicenseActivated(LicenseActivated {
            tenant_id: tenant_id.clone(),
            license_id,
            tier,
            valid_from_ms: issued_at,
            valid_until_ms,
        }));
        self.store.append(&[event]).await?;
        self.get_active(&tenant_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("license not found after activate".to_owned()))
    }

    async fn get_active(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Option<LicenseRecord>, RuntimeError> {
        Ok(LicenseReadModel::get_active(self.store.as_ref(), tenant_id).await?)
    }

    async fn set_override(
        &self,
        tenant_id: TenantId,
        feature: String,
        allowed: bool,
        reason: Option<String>,
    ) -> Result<EntitlementOverrideRecord, RuntimeError> {
        let set_at_ms = now_ms();
        let event = make_envelope(RuntimeEvent::EntitlementOverrideSet(
            EntitlementOverrideSet {
                tenant_id: tenant_id.clone(),
                feature: feature.clone(),
                allowed,
                reason: reason.clone(),
                set_at_ms,
            },
        ));
        self.store.append(&[event]).await?;

        LicenseReadModel::list_overrides(self.store.as_ref(), &tenant_id)
            .await?
            .into_iter()
            .find(|record| record.feature == feature)
            .ok_or_else(|| RuntimeError::Internal("override not found after set".to_owned()))
    }

    async fn check_feature(
        &self,
        tenant_id: &TenantId,
        feature: &str,
    ) -> Result<FeatureGateResult, RuntimeError> {
        let Some(license) = self.get_active(tenant_id).await? else {
            return Ok(FeatureGateResult::Denied {
                reason: format!("no active license for tenant '{}'", tenant_id.as_str()),
            });
        };

        let mut entitlements = EntitlementSet::new(license.tenant_id.clone(), license.tier);
        for entitlement in &license.entitlements {
            entitlements = entitlements.with_entitlement(*entitlement);
        }

        let base = self.feature_gate.check(&entitlements, feature);
        let override_record = LicenseReadModel::list_overrides(self.store.as_ref(), tenant_id)
            .await?
            .into_iter()
            .find(|record| record.feature == feature);

        Ok(match override_record {
            Some(record) if record.allowed => FeatureGateResult::Allowed,
            Some(record) => FeatureGateResult::Denied {
                reason: record
                    .reason
                    .unwrap_or_else(|| format!("feature '{}' disabled by override", feature)),
            },
            None => base,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::{FeatureGateResult, ProductTier, TenantId};
    use cairn_store::InMemoryStore;

    use crate::licenses::LicenseService;
    use crate::services::LicenseServiceImpl;

    #[tokio::test]
    async fn license_activate_and_override_changes_feature_check() {
        let store = Arc::new(InMemoryStore::new());
        let service = LicenseServiceImpl::new(store);
        let tenant_id = TenantId::new("tenant_acme");

        let license = service
            .activate(tenant_id.clone(), ProductTier::LocalEval, None)
            .await
            .unwrap();
        assert_eq!(license.tier, ProductTier::LocalEval);

        let denied = service
            .check_feature(&tenant_id, "advanced_audit_export")
            .await
            .unwrap();
        assert!(matches!(denied, FeatureGateResult::Denied { .. }));

        service
            .set_override(
                tenant_id.clone(),
                "advanced_audit_export".to_owned(),
                true,
                Some("manager override".to_owned()),
            )
            .await
            .unwrap();

        let allowed = service
            .check_feature(&tenant_id, "advanced_audit_export")
            .await
            .unwrap();
        assert_eq!(allowed, FeatureGateResult::Allowed);
    }
}
