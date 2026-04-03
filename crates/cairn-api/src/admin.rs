use async_trait::async_trait;

use crate::http::ListResponse;

/// Admin endpoints for tenant/workspace/project management.
#[async_trait]
pub trait AdminEndpoints: Send + Sync {
    type Error;
    async fn list_tenants(&self) -> Result<ListResponse<serde_json::Value>, Self::Error>;
    async fn list_workspaces(
        &self,
        tenant_id: &str,
    ) -> Result<ListResponse<serde_json::Value>, Self::Error>;
    async fn list_projects(
        &self,
        tenant_id: &str,
        workspace_id: &str,
    ) -> Result<ListResponse<serde_json::Value>, Self::Error>;
}
