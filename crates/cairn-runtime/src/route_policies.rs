//! Tenant-scoped route policy service boundary for RFC 009.

use async_trait::async_trait;
use cairn_domain::providers::{RoutePolicy, RoutePolicyRule};
use cairn_domain::TenantId;

use crate::error::RuntimeError;

#[async_trait]
pub trait RoutePolicyService: Send + Sync {
    async fn create(
        &self,
        tenant_id: TenantId,
        name: String,
        rules: Vec<RoutePolicyRule>,
    ) -> Result<RoutePolicy, RuntimeError>;

    async fn get(&self, policy_id: &str) -> Result<Option<RoutePolicy>, RuntimeError>;
}
