//! Prompt release service boundary per RFC 006.

use async_trait::async_trait;
use cairn_domain::{ProjectKey, PromptAssetId, PromptReleaseId, PromptVersionId};
use cairn_store::projections::PromptReleaseRecord;

use crate::error::RuntimeError;

/// Prompt release service boundary.
///
/// Per RFC 006, prompt releases are project-scoped, selector-targeted,
/// and follow a governed lifecycle (draft -> proposed -> approved -> active).
#[async_trait]
pub trait PromptReleaseService: Send + Sync {
    /// Create a new prompt release in draft state.
    async fn create(
        &self,
        project: &ProjectKey,
        release_id: PromptReleaseId,
        asset_id: PromptAssetId,
        version_id: PromptVersionId,
    ) -> Result<PromptReleaseRecord, RuntimeError>;

    /// Transition a release to a new state.
    async fn transition(
        &self,
        release_id: &PromptReleaseId,
        to_state: &str,
    ) -> Result<PromptReleaseRecord, RuntimeError>;

    /// RFC 006: attach an approval policy to a release.
    /// Subsequent activate() calls will be blocked until approval is granted.
    async fn attach_approval_policy(
        &self,
        release_id: &PromptReleaseId,
        policy_id: &str,
    ) -> Result<(), RuntimeError>;

    /// RFC 006: request approval for a release (emits ApprovalRequested).
    async fn request_approval(
        &self,
        release_id: &PromptReleaseId,
    ) -> Result<cairn_store::projections::ApprovalRecord, RuntimeError>;

    /// RFC 001: start a gradual traffic rollout at the given percentage.
    async fn start_rollout(
        &self,
        release_id: &PromptReleaseId,
        percent: u8,
    ) -> Result<PromptReleaseRecord, RuntimeError>;

    /// Activate a release (deactivates any previously active release for the same asset).
    async fn activate(
        &self,
        release_id: &PromptReleaseId,
    ) -> Result<PromptReleaseRecord, RuntimeError>;

    /// Rollback: deactivate current, reactivate target.
    async fn rollback(
        &self,
        current_id: &PromptReleaseId,
        target_id: &PromptReleaseId,
    ) -> Result<PromptReleaseRecord, RuntimeError>;

    /// Resolve the active release for an asset given a selector context.
    async fn resolve(
        &self,
        project: &ProjectKey,
        asset_id: &PromptAssetId,
        selector: &str,
    ) -> Result<Option<PromptReleaseRecord>, RuntimeError>;

    /// Get a release by ID.
    async fn get(
        &self,
        release_id: &PromptReleaseId,
    ) -> Result<Option<PromptReleaseRecord>, RuntimeError>;

    /// List releases for a project.
    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<PromptReleaseRecord>, RuntimeError>;
}
