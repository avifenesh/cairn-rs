use async_trait::async_trait;
use cairn_domain::ProjectKey;
use cairn_store::projections::{PromptAssetRecord, PromptReleaseRecord, PromptVersionRecord};

use crate::endpoints::ListQuery;
use crate::http::ListResponse;

/// Prompt management endpoints per RFC 010.
#[async_trait]
pub trait PromptEndpoints: Send + Sync {
    type Error;
    async fn list_assets(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<PromptAssetRecord>, Self::Error>;
    async fn list_versions(
        &self,
        asset_id: &str,
        query: &ListQuery,
    ) -> Result<ListResponse<PromptVersionRecord>, Self::Error>;
    async fn list_releases(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<PromptReleaseRecord>, Self::Error>;
}
