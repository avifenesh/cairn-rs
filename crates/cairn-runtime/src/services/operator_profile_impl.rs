//! Concrete operator profile service implementation.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::org::{validate_operator_preferences, OperatorProfile};
use cairn_domain::*;
use cairn_store::projections::{OperatorProfileReadModel, TenantReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::operator_profiles::OperatorProfileService;

pub struct OperatorProfileServiceImpl<S> {
    store: Arc<S>,
}

impl<S> OperatorProfileServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<S> OperatorProfileService for OperatorProfileServiceImpl<S>
where
    S: EventLog + OperatorProfileReadModel + TenantReadModel + Send + Sync + 'static,
{
    async fn create(
        &self,
        tenant_id: TenantId,
        display_name: String,
        email: String,
        role: WorkspaceRole,
    ) -> Result<OperatorProfile, RuntimeError> {
        if TenantReadModel::get(self.store.as_ref(), &tenant_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "tenant",
                id: tenant_id.to_string(),
            });
        }

        let profile_id = OperatorId::new(format!("op_{}", now_ms()));
        let event = make_envelope(RuntimeEvent::OperatorProfileCreated(
            OperatorProfileCreated {
                tenant_id,
                profile_id: profile_id.clone(),
                display_name,
                email,
                role,
            },
        ));
        self.store.append(&[event]).await?;

        OperatorProfileReadModel::get(self.store.as_ref(), &profile_id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("operator profile not found after create".to_owned())
            })
    }

    async fn get(&self, profile_id: &OperatorId) -> Result<Option<OperatorProfile>, RuntimeError> {
        Ok(OperatorProfileReadModel::get(self.store.as_ref(), profile_id).await?)
    }

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<OperatorProfile>, RuntimeError> {
        Ok(
            OperatorProfileReadModel::list_by_tenant(self.store.as_ref(), tenant_id, limit, offset)
                .await?,
        )
    }

    async fn update(
        &self,
        profile_id: &OperatorId,
        display_name: String,
        email: String,
    ) -> Result<OperatorProfile, RuntimeError> {
        let existing = OperatorProfileReadModel::get(self.store.as_ref(), profile_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "operator_profile",
                id: profile_id.to_string(),
            })?;

        let event = make_envelope(RuntimeEvent::OperatorProfileUpdated(
            OperatorProfileUpdated {
                tenant_id: existing.tenant_id.clone(),
                profile_id: profile_id.clone(),
                display_name,
                email,
            },
        ));
        self.store.append(&[event]).await?;

        OperatorProfileReadModel::get(self.store.as_ref(), profile_id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("operator profile not found after update".to_owned())
            })
    }

    async fn set_preferences(
        &self,
        profile_id: &OperatorId,
        preferences: serde_json::Value,
    ) -> Result<OperatorProfile, RuntimeError> {
        // RFC 008: reject preferences that would silently affect runtime outcomes.
        validate_operator_preferences(&preferences).map_err(|reason| {
            RuntimeError::Validation { reason }
        })?;

        let existing = OperatorProfileReadModel::get(self.store.as_ref(), profile_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "operator_profile",
                id: profile_id.to_string(),
            })?;

        // Store preferences by emitting an update event with the same display/email
        // but different preferences (preferences are stored in the in-memory projection).
        let event = make_envelope(RuntimeEvent::OperatorProfileUpdated(
            OperatorProfileUpdated {
                tenant_id: existing.tenant_id.clone(),
                profile_id: profile_id.clone(),
                display_name: existing.display_name.clone(),
                email: existing.email.clone(),
            },
        ));
        self.store.append(&[event]).await?;

        // Return the profile with updated preferences (in-memory store merges them).
        let mut updated = OperatorProfileReadModel::get(self.store.as_ref(), profile_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("profile not found after set_preferences".into()))?;
        updated.preferences = preferences;
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::{TenantId, WorkspaceRole};
    use cairn_store::InMemoryStore;

    use crate::operator_profiles::OperatorProfileService;
    use crate::services::{OperatorProfileServiceImpl, TenantServiceImpl};
    use crate::tenants::TenantService;

    /// RFC 008: operator preferences MUST NOT contain runtime-affecting keys.
    #[tokio::test]
    async fn operator_preferences_rejects_runtime_affecting_keys() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        tenant_service
            .create(TenantId::new("t_prefs"), "Tenant".to_owned())
            .await
            .unwrap();

        let service = OperatorProfileServiceImpl::new(store);
        let profile = service
            .create(
                TenantId::new("t_prefs"),
                "Alice".to_owned(),
                "alice@example.com".to_owned(),
                WorkspaceRole::Member,
            )
            .await
            .unwrap();

        // Safe preferences should be accepted.
        let safe = serde_json::json!({ "theme": "dark", "timezone": "UTC" });
        let result = service.set_preferences(&profile.operator_id, safe).await;
        assert!(result.is_ok(), "safe preferences must be accepted");

        // Runtime-affecting preference must be rejected.
        let unsafe_prefs = serde_json::json!({
            "theme": "dark",
            "provider_routing": { "model": "gpt-4" },
        });
        let err = service
            .set_preferences(&profile.operator_id, unsafe_prefs)
            .await
            .unwrap_err();
        assert!(
            matches!(err, crate::error::RuntimeError::Validation { .. }),
            "runtime-affecting preference must return Validation error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn operator_profile_create_get_round_trip() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        tenant_service
            .create(TenantId::new("tenant_acme"), "Acme".to_owned())
            .await
            .unwrap();

        let service = OperatorProfileServiceImpl::new(store);
        let created = service
            .create(
                TenantId::new("tenant_acme"),
                "Avi".to_owned(),
                "avi@example.com".to_owned(),
                WorkspaceRole::Admin,
            )
            .await
            .unwrap();

        let fetched = service.get(&created.operator_id).await.unwrap().unwrap();

        assert_eq!(created, fetched);
        assert_eq!(fetched.display_name, "Avi");
        assert_eq!(fetched.email, "avi@example.com");
        assert_eq!(fetched.role, WorkspaceRole::Admin);
    }
}
