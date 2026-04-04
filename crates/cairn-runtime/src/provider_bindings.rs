//! Provider binding service boundary for project-scoped routing bindings.

use async_trait::async_trait;
use cairn_domain::providers::{OperationKind, ProviderBindingRecord};
use cairn_domain::{
    ProjectKey, ProviderBindingId, ProviderConnectionId, ProviderModelId, TenantId,
};

use crate::error::RuntimeError;

#[async_trait]
pub trait ProviderBindingService: Send + Sync {
    async fn create(
        &self,
        project: ProjectKey,
        provider_connection_id: ProviderConnectionId,
        operation_kind: OperationKind,
        provider_model_id: ProviderModelId,
        estimated_cost_micros: Option<u64>,
    ) -> Result<ProviderBindingRecord, RuntimeError>;

    async fn get(
        &self,
        id: &ProviderBindingId,
    ) -> Result<Option<ProviderBindingRecord>, RuntimeError>;

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProviderBindingRecord>, RuntimeError>;

    async fn activate(&self, id: &ProviderBindingId)
        -> Result<ProviderBindingRecord, RuntimeError>;

    async fn deactivate(
        &self,
        id: &ProviderBindingId,
    ) -> Result<ProviderBindingRecord, RuntimeError>;
}
