//! Selector resolution service per RFC 006.
//!
//! Resolves which active prompt release to use given a runtime context.
//! Precedence: routing_slot > task_type > agent_type > project_default.

use cairn_domain::{ProjectId, PromptAssetId};

use crate::prompts::{PromptRelease, PromptReleaseState};
use crate::selectors::ResolutionContext;

/// Resolves the active prompt release for a given runtime context.
///
/// Given a set of active releases for a project/asset pair, returns
/// the one with the highest-precedence matching selector.
pub struct SelectorResolver;

impl SelectorResolver {
    /// Resolve the best matching active release.
    ///
    /// Returns `None` if no active release matches the context.
    pub fn resolve<'a>(
        releases: &'a [PromptRelease],
        project_id: &ProjectId,
        prompt_asset_id: &PromptAssetId,
        ctx: &ResolutionContext,
    ) -> Option<&'a PromptRelease> {
        let mut candidates: Vec<&PromptRelease> = releases
            .iter()
            .filter(|r| {
                r.project_id == *project_id
                    && r.prompt_asset_id == *prompt_asset_id
                    && r.state == PromptReleaseState::Active
                    && r.rollout_target.matches(ctx)
            })
            .collect();

        // Sort by precedence descending (highest first)
        candidates.sort_by(|a, b| {
            b.rollout_target
                .kind
                .precedence()
                .cmp(&a.rollout_target.kind.precedence())
        });

        candidates.first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::releases::PromptReleaseState;
    use crate::selectors::RolloutTarget;
    use crate::selectors::SelectorKind;
    use cairn_domain::{ProjectId, PromptAssetId, PromptReleaseId, PromptVersionId};

    fn make_release(id: &str, target: RolloutTarget) -> PromptRelease {
        PromptRelease {
            prompt_release_id: PromptReleaseId::new(id),
            project_id: ProjectId::new("proj_1"),
            prompt_asset_id: PromptAssetId::new("prompt_planner"),
            prompt_version_id: PromptVersionId::new("pv_1"),
            release_tag: None,
            state: PromptReleaseState::Active,
            rollout_target: target,
            created_by: None,
            created_at: 1000,
            updated_at: 1000,
        }
    }

    #[test]
    fn resolves_most_specific_selector() {
        let releases = vec![
            make_release("rel_default", RolloutTarget::project_default()),
            make_release("rel_agent", RolloutTarget::agent_type("planner")),
            make_release("rel_slot", RolloutTarget::routing_slot("fallback_1")),
        ];

        let ctx = ResolutionContext {
            agent_type: Some("planner".to_owned()),
            task_type: None,
            routing_slot: Some("fallback_1".to_owned()),
        };

        let resolved = SelectorResolver::resolve(
            &releases,
            &ProjectId::new("proj_1"),
            &PromptAssetId::new("prompt_planner"),
            &ctx,
        );

        assert_eq!(
            resolved.unwrap().prompt_release_id,
            PromptReleaseId::new("rel_slot")
        );
    }

    #[test]
    fn falls_back_to_project_default() {
        let releases = vec![
            make_release("rel_default", RolloutTarget::project_default()),
            make_release("rel_agent", RolloutTarget::agent_type("coder")),
        ];

        let ctx = ResolutionContext {
            agent_type: Some("planner".to_owned()),
            ..Default::default()
        };

        let resolved = SelectorResolver::resolve(
            &releases,
            &ProjectId::new("proj_1"),
            &PromptAssetId::new("prompt_planner"),
            &ctx,
        );

        assert_eq!(
            resolved.unwrap().prompt_release_id,
            PromptReleaseId::new("rel_default")
        );
    }

    #[test]
    fn returns_none_when_no_match() {
        let releases = vec![make_release(
            "rel_agent",
            RolloutTarget::agent_type("coder"),
        )];

        let ctx = ResolutionContext {
            agent_type: Some("planner".to_owned()),
            ..Default::default()
        };

        let resolved = SelectorResolver::resolve(
            &releases,
            &ProjectId::new("proj_1"),
            &PromptAssetId::new("prompt_planner"),
            &ctx,
        );

        assert!(resolved.is_none());
    }

    #[test]
    fn ignores_non_active_releases() {
        let mut release = make_release("rel_1", RolloutTarget::project_default());
        release.state = PromptReleaseState::Approved; // not active

        let releases = vec![release];

        let ctx = ResolutionContext::default();

        let resolved = SelectorResolver::resolve(
            &releases,
            &ProjectId::new("proj_1"),
            &PromptAssetId::new("prompt_planner"),
            &ctx,
        );

        assert!(resolved.is_none());
    }

    #[test]
    fn task_type_beats_agent_type() {
        let releases = vec![
            make_release("rel_agent", RolloutTarget::agent_type("planner")),
            make_release("rel_task", RolloutTarget::task_type("review")),
        ];

        let ctx = ResolutionContext {
            agent_type: Some("planner".to_owned()),
            task_type: Some("review".to_owned()),
            routing_slot: None,
        };

        let resolved = SelectorResolver::resolve(
            &releases,
            &ProjectId::new("proj_1"),
            &PromptAssetId::new("prompt_planner"),
            &ctx,
        );

        assert_eq!(
            resolved.unwrap().prompt_release_id,
            PromptReleaseId::new("rel_task")
        );
        assert_eq!(
            resolved.unwrap().rollout_target.kind,
            SelectorKind::TaskType
        );
    }
}
