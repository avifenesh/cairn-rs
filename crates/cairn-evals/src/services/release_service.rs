//! Prompt release lifecycle service.
//!
//! Implements state transitions per RFC 006. Enforces transition
//! validation, activation uniqueness, and rollback semantics.

use cairn_domain::{
    OperatorId, ProjectId, PromptAssetId, PromptReleaseId, PromptVersionId, ReleaseActionId,
};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::prompts::{PromptRelease, PromptReleaseState, ReleaseAction, ReleaseActionType};
use crate::selectors::RolloutTarget;

/// Error from release lifecycle operations.
#[derive(Debug)]
pub enum ReleaseError {
    NotFound(String),
    InvalidTransition {
        from: PromptReleaseState,
        to: PromptReleaseState,
    },
    DuplicateActive {
        project_id: String,
        prompt_asset_id: String,
    },
}

impl std::fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReleaseError::NotFound(id) => write!(f, "release not found: {id}"),
            ReleaseError::InvalidTransition { from, to } => {
                write!(f, "invalid transition: {from:?} -> {to:?}")
            }
            ReleaseError::DuplicateActive {
                project_id,
                prompt_asset_id,
            } => {
                write!(
                    f,
                    "active release already exists for {project_id}/{prompt_asset_id}"
                )
            }
        }
    }
}

impl std::error::Error for ReleaseError {}

struct ReleaseState {
    releases: HashMap<String, PromptRelease>,
    actions: Vec<ReleaseAction>,
    next_action_seq: u64,
}

/// In-memory prompt release lifecycle service.
///
/// Implements RFC 006 state transitions:
/// - draft -> proposed | approved | archived
/// - proposed -> approved | rejected | archived
/// - approved -> active | archived
/// - active -> approved | archived
/// - rejected -> archived
pub struct PromptReleaseService {
    state: Mutex<ReleaseState>,
}

impl PromptReleaseService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ReleaseState {
                releases: HashMap::new(),
                actions: Vec::new(),
                next_action_seq: 1,
            }),
        }
    }

    /// Create a new prompt release in Draft state.
    pub fn create(
        &self,
        release_id: PromptReleaseId,
        project_id: ProjectId,
        prompt_asset_id: PromptAssetId,
        prompt_version_id: PromptVersionId,
        rollout_target: RolloutTarget,
    ) -> PromptRelease {
        let now = now_millis();
        let release = PromptRelease {
            prompt_release_id: release_id.clone(),
            project_id,
            prompt_asset_id,
            prompt_version_id,
            release_tag: None,
            state: PromptReleaseState::Draft,
            rollout_target,
            created_by: None,
            created_at: now,
            updated_at: now,
        };

        let mut state = self.state.lock().unwrap();
        state
            .releases
            .insert(release_id.as_str().to_owned(), release.clone());
        release
    }

    /// Transition a release to a new state.
    pub fn transition(
        &self,
        release_id: &PromptReleaseId,
        to: PromptReleaseState,
        actor: Option<OperatorId>,
        reason: Option<String>,
    ) -> Result<PromptRelease, ReleaseError> {
        let mut state = self.state.lock().unwrap();

        // Read and validate, then drop the borrow before mutating
        let snapshot = {
            let release = state
                .releases
                .get(release_id.as_str())
                .ok_or_else(|| ReleaseError::NotFound(release_id.to_string()))?;
            if !release.state.can_transition_to(to) {
                return Err(ReleaseError::InvalidTransition {
                    from: release.state,
                    to,
                });
            }
            release.clone()
        };

        let action_type = match to {
            PromptReleaseState::Proposed => ReleaseActionType::Propose,
            PromptReleaseState::Approved => ReleaseActionType::Approve,
            PromptReleaseState::Rejected => ReleaseActionType::Reject,
            PromptReleaseState::Active => ReleaseActionType::Activate,
            PromptReleaseState::Archived => ReleaseActionType::Archive,
            PromptReleaseState::Draft => {
                return Err(ReleaseError::InvalidTransition {
                    from: snapshot.state,
                    to,
                })
            }
        };

        // Activation: deactivate existing active release for same tuple.
        let mut from_release_id = None;
        if to == PromptReleaseState::Active {
            let release_id_str = release_id.as_str();
            for r in state.releases.values_mut() {
                if r.prompt_release_id.as_str() != release_id_str
                    && r.project_id == snapshot.project_id
                    && r.prompt_asset_id == snapshot.prompt_asset_id
                    && r.rollout_target == snapshot.rollout_target
                    && r.state == PromptReleaseState::Active
                {
                    from_release_id = Some(r.prompt_release_id.clone());
                    r.state = PromptReleaseState::Approved;
                    r.updated_at = now_millis();
                    break;
                }
            }
        }

        let now = now_millis();
        let seq = state.next_action_seq;
        state.next_action_seq += 1;

        {
            let release = state.releases.get_mut(release_id.as_str()).unwrap();
            release.state = to;
            release.updated_at = now;
        }

        state.actions.push(ReleaseAction {
            release_action_id: ReleaseActionId::new(format!("ra_{seq}")),
            prompt_release_id: release_id.clone(),
            action_type,
            actor,
            reason,
            from_release_id,
            to_release_id: Some(release_id.clone()),
            created_at: now,
        });

        Ok(state.releases.get(release_id.as_str()).unwrap().clone())
    }

    /// Rollback: re-activate a previously approved release.
    ///
    /// Per RFC 006: rollback re-activates a prior approved release for
    /// the same project/asset/selector tuple. It does not create a new
    /// release object.
    pub fn rollback(
        &self,
        current_active_id: &PromptReleaseId,
        target_release_id: &PromptReleaseId,
        actor: Option<OperatorId>,
        reason: Option<String>,
    ) -> Result<PromptRelease, ReleaseError> {
        let mut state = self.state.lock().unwrap();

        // Validate states (read-only, then drop borrows)
        {
            let current = state
                .releases
                .get(current_active_id.as_str())
                .ok_or_else(|| ReleaseError::NotFound(current_active_id.to_string()))?;
            if current.state != PromptReleaseState::Active {
                return Err(ReleaseError::InvalidTransition {
                    from: current.state,
                    to: PromptReleaseState::Approved,
                });
            }
        }
        {
            let target = state
                .releases
                .get(target_release_id.as_str())
                .ok_or_else(|| ReleaseError::NotFound(target_release_id.to_string()))?;
            if target.state != PromptReleaseState::Approved {
                return Err(ReleaseError::InvalidTransition {
                    from: target.state,
                    to: PromptReleaseState::Active,
                });
            }
        }

        let now = now_millis();

        // Deactivate current
        state
            .releases
            .get_mut(current_active_id.as_str())
            .unwrap()
            .state = PromptReleaseState::Approved;
        state
            .releases
            .get_mut(current_active_id.as_str())
            .unwrap()
            .updated_at = now;

        // Activate target
        state
            .releases
            .get_mut(target_release_id.as_str())
            .unwrap()
            .state = PromptReleaseState::Active;
        state
            .releases
            .get_mut(target_release_id.as_str())
            .unwrap()
            .updated_at = now;

        let seq = state.next_action_seq;
        state.next_action_seq += 1;

        state.actions.push(ReleaseAction {
            release_action_id: ReleaseActionId::new(format!("ra_{seq}")),
            prompt_release_id: target_release_id.clone(),
            action_type: ReleaseActionType::Rollback,
            actor,
            reason,
            from_release_id: Some(current_active_id.clone()),
            to_release_id: Some(target_release_id.clone()),
            created_at: now,
        });

        Ok(state
            .releases
            .get(target_release_id.as_str())
            .unwrap()
            .clone())
    }

    /// Get a release by ID.
    pub fn get(&self, release_id: &PromptReleaseId) -> Option<PromptRelease> {
        let state = self.state.lock().unwrap();
        state.releases.get(release_id.as_str()).cloned()
    }

    /// List release actions for audit.
    pub fn actions(&self) -> Vec<ReleaseAction> {
        let state = self.state.lock().unwrap();
        state.actions.clone()
    }
}

impl Default for PromptReleaseService {
    fn default() -> Self {
        Self::new()
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc() -> PromptReleaseService {
        PromptReleaseService::new()
    }

    #[test]
    fn full_approval_lifecycle() {
        let svc = svc();
        let release = svc.create(
            PromptReleaseId::new("rel_1"),
            ProjectId::new("proj_1"),
            PromptAssetId::new("prompt_planner"),
            PromptVersionId::new("pv_1"),
            RolloutTarget::project_default(),
        );
        assert_eq!(release.state, PromptReleaseState::Draft);

        let release = svc
            .transition(
                &PromptReleaseId::new("rel_1"),
                PromptReleaseState::Proposed,
                None,
                None,
            )
            .unwrap();
        assert_eq!(release.state, PromptReleaseState::Proposed);

        let release = svc
            .transition(
                &PromptReleaseId::new("rel_1"),
                PromptReleaseState::Approved,
                None,
                None,
            )
            .unwrap();
        assert_eq!(release.state, PromptReleaseState::Approved);

        let release = svc
            .transition(
                &PromptReleaseId::new("rel_1"),
                PromptReleaseState::Active,
                None,
                None,
            )
            .unwrap();
        assert_eq!(release.state, PromptReleaseState::Active);
    }

    #[test]
    fn activation_deactivates_previous() {
        let svc = svc();
        let target = RolloutTarget::project_default();

        // Release A: draft -> approved -> active
        svc.create(
            PromptReleaseId::new("rel_a"),
            ProjectId::new("proj_1"),
            PromptAssetId::new("prompt_planner"),
            PromptVersionId::new("pv_1"),
            target.clone(),
        );
        svc.transition(
            &PromptReleaseId::new("rel_a"),
            PromptReleaseState::Approved,
            None,
            None,
        )
        .unwrap();
        svc.transition(
            &PromptReleaseId::new("rel_a"),
            PromptReleaseState::Active,
            None,
            None,
        )
        .unwrap();

        // Release B: draft -> approved -> active (should deactivate A)
        svc.create(
            PromptReleaseId::new("rel_b"),
            ProjectId::new("proj_1"),
            PromptAssetId::new("prompt_planner"),
            PromptVersionId::new("pv_2"),
            target,
        );
        svc.transition(
            &PromptReleaseId::new("rel_b"),
            PromptReleaseState::Approved,
            None,
            None,
        )
        .unwrap();
        svc.transition(
            &PromptReleaseId::new("rel_b"),
            PromptReleaseState::Active,
            None,
            None,
        )
        .unwrap();

        let a = svc.get(&PromptReleaseId::new("rel_a")).unwrap();
        let b = svc.get(&PromptReleaseId::new("rel_b")).unwrap();
        assert_eq!(a.state, PromptReleaseState::Approved);
        assert_eq!(b.state, PromptReleaseState::Active);
    }

    #[test]
    fn invalid_transition_rejected() {
        let svc = svc();
        svc.create(
            PromptReleaseId::new("rel_1"),
            ProjectId::new("proj_1"),
            PromptAssetId::new("prompt_planner"),
            PromptVersionId::new("pv_1"),
            RolloutTarget::project_default(),
        );

        // Can't go draft -> active (must go through approved)
        let result = svc.transition(
            &PromptReleaseId::new("rel_1"),
            PromptReleaseState::Active,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn rollback_swaps_active_releases() {
        let svc = svc();
        let target = RolloutTarget::project_default();

        // Release A: draft -> approved -> active
        svc.create(
            PromptReleaseId::new("rel_a"),
            ProjectId::new("proj_1"),
            PromptAssetId::new("prompt_planner"),
            PromptVersionId::new("pv_1"),
            target.clone(),
        );
        svc.transition(
            &PromptReleaseId::new("rel_a"),
            PromptReleaseState::Approved,
            None,
            None,
        )
        .unwrap();
        svc.transition(
            &PromptReleaseId::new("rel_a"),
            PromptReleaseState::Active,
            None,
            None,
        )
        .unwrap();

        // Release B: draft -> approved -> active (A goes to approved)
        svc.create(
            PromptReleaseId::new("rel_b"),
            ProjectId::new("proj_1"),
            PromptAssetId::new("prompt_planner"),
            PromptVersionId::new("pv_2"),
            target,
        );
        svc.transition(
            &PromptReleaseId::new("rel_b"),
            PromptReleaseState::Approved,
            None,
            None,
        )
        .unwrap();
        svc.transition(
            &PromptReleaseId::new("rel_b"),
            PromptReleaseState::Active,
            None,
            None,
        )
        .unwrap();

        // Rollback B -> A
        let restored = svc
            .rollback(
                &PromptReleaseId::new("rel_b"),
                &PromptReleaseId::new("rel_a"),
                None,
                Some("regression found".to_owned()),
            )
            .unwrap();

        assert_eq!(restored.state, PromptReleaseState::Active);
        assert_eq!(restored.prompt_release_id, PromptReleaseId::new("rel_a"));

        let b = svc.get(&PromptReleaseId::new("rel_b")).unwrap();
        assert_eq!(b.state, PromptReleaseState::Approved);

        // Verify audit trail
        let actions = svc.actions();
        let rollback_action = actions
            .iter()
            .find(|a| a.action_type == ReleaseActionType::Rollback)
            .unwrap();
        assert_eq!(
            rollback_action.from_release_id.as_ref().unwrap().as_str(),
            "rel_b"
        );
        assert_eq!(
            rollback_action.to_release_id.as_ref().unwrap().as_str(),
            "rel_a"
        );
    }
}
