use crate::ids::{OperatorId, ProjectId, TenantId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Starter template category per RFC 012.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StarterTemplateCategory {
    KnowledgeAssistant,
    ApprovalGatedWorker,
    MultiStepWorkflow,
}

/// A shipped starter template definition (system-scoped).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StarterTemplate {
    pub id: String,
    pub category: StarterTemplateCategory,
    pub name: String,
    pub description: String,
    /// Prompt asset names included in this template.
    pub prompt_assets: Vec<String>,
    /// Policy preset names included.
    pub policy_presets: Vec<String>,
    /// Skill pack names included.
    pub skill_packs: Vec<String>,
}

/// Onboarding flow state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingFlowState {
    NotStarted,
    InProgress,
    Completed,
    Failed,
}

/// Individual onboarding step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingStep {
    pub step_id: String,
    pub name: String,
    pub completed: bool,
}

/// Tracks onboarding progress for a project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnboardingProgress {
    pub project_id: ProjectId,
    pub template_id: Option<String>,
    pub state: OnboardingFlowState,
    pub steps: Vec<OnboardingStep>,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
}

/// Records provenance of what was materialized during bootstrap.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BootstrapProvenance {
    pub project_id: ProjectId,
    pub tenant_id: TenantId,
    pub workspace_id: WorkspaceId,
    pub template_id: String,
    pub template_category: StarterTemplateCategory,
    pub materialized_at: u64,
    pub materialized_by: Option<OperatorId>,
    /// What was created from the template.
    pub materialized_assets: Vec<MaterializedAsset>,
}

/// One asset materialized from a starter template.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterializedAsset {
    pub asset_type: String,
    pub asset_id: String,
    pub source_template_ref: String,
    pub diverged: bool,
}

/// Import provenance record per RFC 012.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportProvenanceRecord {
    pub import_id: String,
    pub source: String,
    pub imported_at: u64,
    pub bundle_ref: Option<String>,
    pub items: Vec<ImportItem>,
}

/// Individual item in an import operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportItem {
    pub asset_type: String,
    pub asset_id: String,
    pub outcome: ImportOutcome,
}

/// Outcome of importing a single item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportOutcome {
    Created,
    Reused,
    Skipped,
    Conflicted,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_template_categories_are_distinct() {
        assert_ne!(
            StarterTemplateCategory::KnowledgeAssistant,
            StarterTemplateCategory::ApprovalGatedWorker,
        );
    }

    #[test]
    fn onboarding_progress_tracks_steps() {
        let progress = OnboardingProgress {
            project_id: ProjectId::new("p1"),
            template_id: Some("knowledge-assistant".to_owned()),
            state: OnboardingFlowState::InProgress,
            steps: vec![
                OnboardingStep {
                    step_id: "create_project".to_owned(),
                    name: "Create project".to_owned(),
                    completed: true,
                },
                OnboardingStep {
                    step_id: "configure_provider".to_owned(),
                    name: "Configure provider".to_owned(),
                    completed: false,
                },
            ],
            started_at: Some(1000),
            completed_at: None,
        };
        assert_eq!(progress.steps.len(), 2);
        assert!(progress.steps[0].completed);
        assert!(!progress.steps[1].completed);
    }

    #[test]
    fn import_outcomes_are_distinct() {
        assert_ne!(ImportOutcome::Created, ImportOutcome::Reused);
        assert_ne!(ImportOutcome::Skipped, ImportOutcome::Conflicted);
    }
}
