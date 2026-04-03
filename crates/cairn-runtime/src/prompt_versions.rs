//! Prompt version service boundary per RFC 006.

use async_trait::async_trait;
use cairn_domain::{ProjectKey, PromptAssetId, PromptVersionId};
use cairn_store::projections::PromptVersionRecord;

use crate::error::RuntimeError;

#[async_trait]
pub trait PromptVersionService: Send + Sync {
    async fn create(
        &self,
        project: &ProjectKey,
        prompt_version_id: PromptVersionId,
        prompt_asset_id: PromptAssetId,
        content_hash: String,
    ) -> Result<PromptVersionRecord, RuntimeError>;

    async fn get(
        &self,
        prompt_version_id: &PromptVersionId,
    ) -> Result<Option<PromptVersionRecord>, RuntimeError>;

    async fn list_by_asset(
        &self,
        prompt_asset_id: &PromptAssetId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<PromptVersionRecord>, RuntimeError>;
}
