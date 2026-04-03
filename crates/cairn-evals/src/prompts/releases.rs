use cairn_domain::{
    OperatorId, ProjectId, PromptAssetId, PromptReleaseId, PromptVersionId, ReleaseActionId,
};
use serde::{Deserialize, Serialize};

use crate::selectors::RolloutTarget;

/// Prompt release lifecycle state per RFC 006.
///
/// There is one canonical lifecycle field. V1 must not introduce a
/// second orthogonal `approval_state` field.
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

/// A prompt release is the deployable runtime binding of a prompt
/// version into a project.
///
/// Per RFC 006: only one active release may exist per
/// project/prompt-asset/rollout-target tuple.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptRelease {
    pub prompt_release_id: PromptReleaseId,
    pub project_id: ProjectId,
    pub prompt_asset_id: PromptAssetId,
    pub prompt_version_id: PromptVersionId,
    pub release_tag: Option<String>,
    pub state: PromptReleaseState,
    pub rollout_target: RolloutTarget,
    pub created_by: Option<OperatorId>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Release action types for auditable transitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseActionType {
    Propose,
    Approve,
    Reject,
    Activate,
    Deactivate,
    Rollback,
    Archive,
}

/// Durable record of a release transition for audit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseAction {
    pub release_action_id: ReleaseActionId,
    pub prompt_release_id: PromptReleaseId,
    pub action_type: ReleaseActionType,
    pub actor: Option<OperatorId>,
    pub reason: Option<String>,
    pub from_release_id: Option<PromptReleaseId>,
    pub to_release_id: Option<PromptReleaseId>,
    pub created_at: u64,
}

impl PromptReleaseState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            PromptReleaseState::Rejected | PromptReleaseState::Archived
        )
    }

    /// Valid transitions per RFC 006.
    pub fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (PromptReleaseState::Draft, PromptReleaseState::Proposed)
                | (PromptReleaseState::Draft, PromptReleaseState::Approved)
                | (PromptReleaseState::Draft, PromptReleaseState::Archived)
                | (PromptReleaseState::Proposed, PromptReleaseState::Approved)
                | (PromptReleaseState::Proposed, PromptReleaseState::Rejected)
                | (PromptReleaseState::Proposed, PromptReleaseState::Archived)
                | (PromptReleaseState::Approved, PromptReleaseState::Active)
                | (PromptReleaseState::Approved, PromptReleaseState::Archived)
                | (PromptReleaseState::Active, PromptReleaseState::Approved)
                | (PromptReleaseState::Active, PromptReleaseState::Archived)
                | (PromptReleaseState::Rejected, PromptReleaseState::Archived)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::PromptReleaseState;

    #[test]
    fn valid_transitions_match_rfc_006() {
        assert!(PromptReleaseState::Draft.can_transition_to(PromptReleaseState::Proposed));
        assert!(PromptReleaseState::Draft.can_transition_to(PromptReleaseState::Approved));
        assert!(PromptReleaseState::Proposed.can_transition_to(PromptReleaseState::Approved));
        assert!(PromptReleaseState::Proposed.can_transition_to(PromptReleaseState::Rejected));
        assert!(PromptReleaseState::Approved.can_transition_to(PromptReleaseState::Active));
        assert!(PromptReleaseState::Active.can_transition_to(PromptReleaseState::Approved));
    }

    #[test]
    fn invalid_transitions_are_rejected() {
        assert!(!PromptReleaseState::Draft.can_transition_to(PromptReleaseState::Active));
        assert!(!PromptReleaseState::Rejected.can_transition_to(PromptReleaseState::Active));
        assert!(!PromptReleaseState::Archived.can_transition_to(PromptReleaseState::Active));
        assert!(!PromptReleaseState::Active.can_transition_to(PromptReleaseState::Draft));
    }

    #[test]
    fn terminal_states_are_correct() {
        assert!(PromptReleaseState::Rejected.is_terminal());
        assert!(PromptReleaseState::Archived.is_terminal());
        assert!(!PromptReleaseState::Active.is_terminal());
        assert!(!PromptReleaseState::Draft.is_terminal());
    }
}
