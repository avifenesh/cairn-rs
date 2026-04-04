//! Concrete provider binding service implementation.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::providers::{OperationKind, ProviderBindingRecord, ProviderBindingSettings};
use cairn_domain::*;
use cairn_store::projections::{
    ProjectReadModel, ProviderBindingReadModel, ProviderConnectionReadModel,
};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::provider_bindings::ProviderBindingService;

pub struct ProviderBindingServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ProviderBindingServiceImpl<S> {
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

fn operation_label(operation: OperationKind) -> &'static str {
    match operation {
        OperationKind::Generate => "generate",
        OperationKind::Embed => "embed",
        OperationKind::Rerank => "rerank",
    }
}

#[async_trait]
impl<S> ProviderBindingService for ProviderBindingServiceImpl<S>
where
    S: EventLog
        + ProviderBindingReadModel
        + ProviderConnectionReadModel
        + ProjectReadModel
        + Send
        + Sync
        + 'static,
{
    async fn create(
        &self,
        project: ProjectKey,
        provider_connection_id: ProviderConnectionId,
        operation_kind: OperationKind,
        provider_model_id: ProviderModelId,
        estimated_cost_micros: Option<u64>,
    ) -> Result<ProviderBindingRecord, RuntimeError> {
        if ProjectReadModel::get_project(self.store.as_ref(), &project)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "project",
                id: project.project_id.to_string(),
            });
        }

        let connection =
            ProviderConnectionReadModel::get(self.store.as_ref(), &provider_connection_id)
                .await?
                .ok_or_else(|| RuntimeError::NotFound {
                    entity: "provider_connection",
                    id: provider_connection_id.to_string(),
                })?;

        if connection.tenant_id != project.tenant_id {
            return Err(RuntimeError::Internal(
                "provider connection tenant does not match binding project".to_owned(),
            ));
        }

        let created_at = now_ms();
        let provider_binding_id = ProviderBindingId::new(format!(
            "pb_{}_{}_{}",
            created_at,
            provider_connection_id.as_str(),
            operation_label(operation_kind)
        ));

        let event = make_envelope(RuntimeEvent::ProviderBindingCreated(
            ProviderBindingCreated {
                project: project.clone(),
                provider_binding_id: provider_binding_id.clone(),
                policy_id: None,
                provider_connection_id,
                provider_model_id,
                operation_kind,
                settings: ProviderBindingSettings::default(),
                active: true,
                created_at,
                estimated_cost_micros,
            },
        ));

        self.store.append(&[event]).await?;

        ProviderBindingReadModel::get(self.store.as_ref(), &provider_binding_id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("provider binding not found after create".to_owned())
            })
    }

    async fn get(
        &self,
        id: &ProviderBindingId,
    ) -> Result<Option<ProviderBindingRecord>, RuntimeError> {
        Ok(ProviderBindingReadModel::get(self.store.as_ref(), id).await?)
    }

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProviderBindingRecord>, RuntimeError> {
        Ok(
            ProviderBindingReadModel::list_by_tenant(self.store.as_ref(), tenant_id, limit, offset)
                .await?,
        )
    }

    async fn activate(
        &self,
        id: &ProviderBindingId,
    ) -> Result<ProviderBindingRecord, RuntimeError> {
        let binding = ProviderBindingReadModel::get(self.store.as_ref(), id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "provider_binding",
                id: id.to_string(),
            })?;

        if binding.active {
            return Ok(binding);
        }

        let event = make_envelope(RuntimeEvent::ProviderBindingStateChanged(
            ProviderBindingStateChanged {
                project: binding.project.clone(),
                provider_binding_id: binding.provider_binding_id.clone(),
                active: true,
                changed_at: now_ms(),
            },
        ));
        self.store.append(&[event]).await?;

        ProviderBindingReadModel::get(self.store.as_ref(), id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("provider binding not found after activate".to_owned())
            })
    }

    async fn deactivate(
        &self,
        id: &ProviderBindingId,
    ) -> Result<ProviderBindingRecord, RuntimeError> {
        let binding = ProviderBindingReadModel::get(self.store.as_ref(), id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "provider_binding",
                id: id.to_string(),
            })?;

        if !binding.active {
            return Ok(binding);
        }

        let event = make_envelope(RuntimeEvent::ProviderBindingStateChanged(
            ProviderBindingStateChanged {
                project: binding.project.clone(),
                provider_binding_id: binding.provider_binding_id.clone(),
                active: false,
                changed_at: now_ms(),
            },
        ));
        self.store.append(&[event]).await?;

        ProviderBindingReadModel::get(self.store.as_ref(), id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("provider binding not found after deactivate".to_owned())
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::providers::OperationKind;
    use cairn_domain::{ProjectKey, ProviderConnectionId, ProviderModelId, TenantId, WorkspaceId};
    use cairn_store::InMemoryStore;

    use crate::projects::ProjectService;
    use crate::provider_bindings::ProviderBindingService;
    use crate::provider_connections::{ProviderConnectionConfig, ProviderConnectionService};
    use crate::services::{
        ProjectServiceImpl, ProviderBindingServiceImpl, ProviderConnectionServiceImpl,
        TenantServiceImpl, WorkspaceServiceImpl,
    };
    use crate::tenants::TenantService;
    use crate::workspaces::WorkspaceService;

    #[tokio::test]
    async fn create_and_get_round_trip() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        let workspace_service = WorkspaceServiceImpl::new(store.clone());
        let project_service = ProjectServiceImpl::new(store.clone());
        let connection_service = ProviderConnectionServiceImpl::new(store.clone());

        tenant_service
            .create(TenantId::new("tenant_acme"), "Acme".to_owned())
            .await
            .unwrap();
        workspace_service
            .create(
                TenantId::new("tenant_acme"),
                WorkspaceId::new("ws_main"),
                "Main".to_owned(),
            )
            .await
            .unwrap();
        let project = ProjectKey::new("tenant_acme", "ws_main", "project_alpha");
        project_service
            .create(project.clone(), "Alpha".to_owned())
            .await
            .unwrap();
        connection_service
            .create(
                TenantId::new("tenant_acme"),
                ProviderConnectionId::new("conn_openai"),
                ProviderConnectionConfig {
                    provider_family: "openai".to_owned(),
                    adapter_type: "responses_api".to_owned(),
                },
            )
            .await
            .unwrap();

        let binding_service = ProviderBindingServiceImpl::new(store);
        let created = binding_service
            .create(
                project,
                ProviderConnectionId::new("conn_openai"),
                OperationKind::Generate,
                ProviderModelId::new("gpt-5"),
                None,
            )
            .await
            .unwrap();

        let fetched = binding_service
            .get(&created.provider_binding_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(created, fetched);
        assert!(fetched.active);
        assert_eq!(fetched.provider_model_id, ProviderModelId::new("gpt-5"));
    }
}
