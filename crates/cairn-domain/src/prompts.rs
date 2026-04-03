use crate::ids::{ProjectId, PromptAssetId, PromptReleaseId, PromptVersionId, ReleaseActionId};
use crate::selectors::RolloutTarget;
use serde::{Deserialize, Serialize};

/// Stable prompt family kinds from RFC 006.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    System,
    UserTemplate,
    ToolPrompt,
    Critic,
    Router,
}

/// Canonical prompt release lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptReleaseState {
    Draft,
    Proposed,
    Approved,
    Active,
    Rejected,
    Archived,
}

/// Auditable prompt release actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseActionType {
    Proposed,
    Approved,
    Rejected,
    Activated,
    RolledBack,
    Archived,
}

/// Shared governance preset used to tighten the transition set for regulated projects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptGovernancePreset {
    Standard,
    Regulated,
}

/// Uniqueness unit for live prompt routing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptReleaseKey {
    pub project_id: ProjectId,
    pub prompt_asset_id: PromptAssetId,
    pub rollout_target: RolloutTarget,
}

/// Minimal release record shared across eval, graph, runtime, and API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptReleaseRecord {
    pub prompt_release_id: PromptReleaseId,
    pub project_id: ProjectId,
    pub prompt_asset_id: PromptAssetId,
    pub prompt_version_id: PromptVersionId,
    pub release_tag: String,
    pub state: PromptReleaseState,
    pub rollout_target: RolloutTarget,
}

/// Minimal release action record for audit linkage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptReleaseActionRecord {
    pub release_action_id: ReleaseActionId,
    pub prompt_release_id: PromptReleaseId,
    pub action_type: ReleaseActionType,
    pub from_release_id: Option<PromptReleaseId>,
    pub to_release_id: Option<PromptReleaseId>,
}

pub fn can_transition_prompt_release(
    from: PromptReleaseState,
    to: PromptReleaseState,
    preset: PromptGovernancePreset,
) -> bool {
    matches!(
        (from, to, preset),
        (PromptReleaseState::Draft, PromptReleaseState::Proposed, _)
            | (
                PromptReleaseState::Draft,
                PromptReleaseState::Approved,
                PromptGovernancePreset::Standard
            )
            | (PromptReleaseState::Draft, PromptReleaseState::Archived, _)
            | (
                PromptReleaseState::Proposed,
                PromptReleaseState::Approved,
                _
            )
            | (
                PromptReleaseState::Proposed,
                PromptReleaseState::Rejected,
                _
            )
            | (
                PromptReleaseState::Proposed,
                PromptReleaseState::Archived,
                _
            )
            | (PromptReleaseState::Approved, PromptReleaseState::Active, _)
            | (
                PromptReleaseState::Approved,
                PromptReleaseState::Archived,
                _
            )
            | (PromptReleaseState::Active, PromptReleaseState::Approved, _)
            | (PromptReleaseState::Active, PromptReleaseState::Archived, _)
            | (
                PromptReleaseState::Rejected,
                PromptReleaseState::Archived,
                _
            )
    )
}

#[cfg(test)]
mod tests {
    use super::{can_transition_prompt_release, PromptGovernancePreset, PromptReleaseState};

    #[test]
    fn regulated_projects_forbid_draft_to_approved_shortcut() {
        assert!(!can_transition_prompt_release(
            PromptReleaseState::Draft,
            PromptReleaseState::Approved,
            PromptGovernancePreset::Regulated,
        ));
    }

    #[test]
    fn standard_projects_allow_draft_to_approved_shortcut() {
        assert!(can_transition_prompt_release(
            PromptReleaseState::Draft,
            PromptReleaseState::Approved,
            PromptGovernancePreset::Standard,
        ));
    }
}
