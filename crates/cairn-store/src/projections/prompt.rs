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
    /// Scope of this asset (e.g. "project", "workspace", "tenant").
    #[serde(default)]
    pub scope: String,
    /// Lifecycle status (e.g. "draft", "published", "archived").
    #[serde(default)]
    pub status: String,
    /// Workspace ID for cross-workspace lookup.
    #[serde(default)]
    pub workspace: String,
    /// Last-updated timestamp.
    #[serde(default)]
    pub updated_at: u64,
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
    /// Monotonically increasing version number within the asset.
    #[serde(default)]
    pub version_number: u32,
    /// Workspace ID for cross-workspace operations.
    #[serde(default)]
    pub workspace: String,
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
    /// RFC 001: percentage of traffic routed to this release (0-100).
    pub rollout_percent: Option<u8>,
    /// RFC 006: routing slot this release is pinned to (e.g. "slot_a").
    /// When set, selectors that match this slot get this release first.
    #[serde(default)]
    pub routing_slot: Option<String>,
    /// RFC 006: task-type affinity (e.g. "worker", "planner").
    /// Second in precedence after routing_slot.
    #[serde(default)]
    pub task_type: Option<String>,
    /// RFC 006: agent-type affinity (e.g. "orchestrator").
    /// Third in precedence after task_type.
    #[serde(default)]
    pub agent_type: Option<String>,
    /// RFC 006: marks this release as the project-wide default fallback.
    #[serde(default)]
    pub is_project_default: bool,
    /// RFC 006: optional human-readable tag (e.g. "v1.2-beta") from the creation event.
    #[serde(default)]
    pub release_tag: Option<String>,
    /// RFC 006: operator or service account that created this release.
    #[serde(default)]
    pub created_by: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[async_trait]
pub trait PromptReleaseReadModel: Send + Sync {
    async fn get(&self, id: &PromptReleaseId) -> Result<Option<PromptReleaseRecord>, StoreError>;
    async fn list_by_project(&self, project: &ProjectKey, limit: usize, offset: usize) -> Result<Vec<PromptReleaseRecord>, StoreError>;
    async fn active_for_selector(&self, project: &ProjectKey, prompt_asset_id: &PromptAssetId, selector: &str) -> Result<Option<PromptReleaseRecord>, StoreError>;
}
