use async_trait::async_trait;
use cairn_domain::providers::{ProviderCallRecord, RouteDecisionRecord};
use cairn_domain::{ProjectKey, RouteDecisionId};

use crate::error::StoreError;

/// Read-model for route decision records.
#[async_trait]
pub trait RouteDecisionReadModel: Send + Sync {
    async fn get(
        &self,
        decision_id: &RouteDecisionId,
    ) -> Result<Option<RouteDecisionRecord>, StoreError>;

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RouteDecisionRecord>, StoreError>;
}

/// Read-model for provider call records.
#[async_trait]
pub trait ProviderCallReadModel: Send + Sync {
    async fn get(
        &self,
        call_id: &cairn_domain::ProviderCallId,
    ) -> Result<Option<ProviderCallRecord>, StoreError>;

    async fn list_by_decision(
        &self,
        decision_id: &RouteDecisionId,
        limit: usize,
    ) -> Result<Vec<ProviderCallRecord>, StoreError>;
}
