//! Prompt asset service boundary per RFC 006.

use async_trait::async_trait;
use cairn_domain::{ProjectKey, PromptAssetId};
use cairn_store::projections::PromptAssetRecord;

use crate::error::RuntimeError;

#[async_trait]
pub trait PromptAssetService: Send + Sync {
    async fn create(
        &self,
        project: &ProjectKey,
        prompt_asset_id: PromptAssetId,
        name: String,
        kind: String,
    ) -> Result<PromptAssetRecord, RuntimeError>;

    async fn get(
        &self,
        prompt_asset_id: &PromptAssetId,
    ) -> Result<Option<PromptAssetRecord>, RuntimeError>;

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<PromptAssetRecord>, RuntimeError>;
}
