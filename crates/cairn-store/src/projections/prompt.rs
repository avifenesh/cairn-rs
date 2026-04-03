use async_trait::async_trait;
use cairn_domain::{PromptAssetId, PromptVersionId, PromptReleaseId, ProjectKey};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptAssetRecord {
    pub prompt_asset_id: PromptAssetId,
    pub project: ProjectKey,
    pub name: String,
    pub kind: String,
    pub created_at: u64,
}

#[async_trait]
pub trait PromptAssetReadModel: Send + Sync {
    async fn get(&self, id: &PromptAssetId) -> Result<Option<PromptAssetRecord>, StoreError>;
    async fn list_by_project(&self, project: &ProjectKey, limit: usize, offset: usize) -> Result<Vec<PromptAssetRecord>, StoreError>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptVersionRecord {
    pub prompt_version_id: PromptVersionId,
    pub prompt_asset_id: PromptAssetId,
    pub project: ProjectKey,
    pub content_hash: String,
    pub created_at: u64,
}

#[async_trait]
pub trait PromptVersionReadModel: Send + Sync {
    async fn get(&self, id: &PromptVersionId) -> Result<Option<PromptVersionRecord>, StoreError>;
    async fn list_by_asset(&self, asset_id: &PromptAssetId, limit: usize, offset: usize) -> Result<Vec<PromptVersionRecord>, StoreError>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptReleaseRecord {
    pub prompt_release_id: PromptReleaseId,
    pub project: ProjectKey,
    pub prompt_asset_id: PromptAssetId,
    pub prompt_version_id: PromptVersionId,
    pub state: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[async_trait]
pub trait PromptReleaseReadModel: Send + Sync {
    async fn get(&self, id: &PromptReleaseId) -> Result<Option<PromptReleaseRecord>, StoreError>;
    async fn list_by_project(&self, project: &ProjectKey, limit: usize, offset: usize) -> Result<Vec<PromptReleaseRecord>, StoreError>;
    async fn active_for_selector(&self, project: &ProjectKey, prompt_asset_id: &PromptAssetId, selector: &str) -> Result<Option<PromptReleaseRecord>, StoreError>;
}
