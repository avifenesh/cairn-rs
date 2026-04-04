//! Onboarding services: starter template registry, materialization, import.
//!
//! Per RFC 012, V1 ships three mandatory starter template categories
//! and a canonical bootstrap path from install to first value.

use cairn_domain::onboarding::{
    BootstrapProvenance, ImportOutcome, ImportProvenanceRecord, MaterializedAsset,
    OnboardingFlowState, OnboardingProgress, OnboardingStep, StarterTemplate,
    StarterTemplateCategory,
};
use cairn_domain::{ProjectId, TenantId, WorkspaceId};

/// System-scoped registry of shipped starter templates.
pub struct StarterTemplateRegistry {
    templates: Vec<StarterTemplate>,
}

impl StarterTemplateRegistry {
    /// Create the canonical V1 registry with the three required templates.
    pub fn v1_defaults() -> Self {
        Self {
            templates: vec![
                StarterTemplate {
                    id: "knowledge-assistant".to_owned(),
                    category: StarterTemplateCategory::KnowledgeAssistant,
                    name: "Knowledge Assistant".to_owned(),
                    description: "Retrieval-aware agent with starter prompts and memory policy"
                        .to_owned(),
                    prompt_assets: vec![
                        "assistant.system".to_owned(),
                        "retrieval.answer".to_owned(),
                    ],
                    policy_presets: vec!["retrieval-default".to_owned()],
                    skill_packs: vec![],
                },
                StarterTemplate {
                    id: "approval-gated-worker".to_owned(),
                    category: StarterTemplateCategory::ApprovalGatedWorker,
                    name: "Approval-Gated Worker".to_owned(),
                    description:
                        "Workflow with approval checkpoints and operator control visibility"
                            .to_owned(),
                    prompt_assets: vec!["worker.system".to_owned()],
                    policy_presets: vec!["approval-required".to_owned()],
                    skill_packs: vec![],
                },
                StarterTemplate {
                    id: "multi-step-workflow".to_owned(),
                    category: StarterTemplateCategory::MultiStepWorkflow,
                    name: "Multi-Step Operator Workflow".to_owned(),
                    description: "Orchestration with tools, stages, and control-plane visibility"
                        .to_owned(),
                    prompt_assets: vec![
                        "planner.system".to_owned(),
                        "executor.system".to_owned(),
                    ],
                    policy_presets: vec!["tool-permission-default".to_owned()],
                    skill_packs: vec![],
                },
            ],
        }
    }

    pub fn list(&self) -> &[StarterTemplate] {
        &self.templates
    }

    pub fn get(&self, template_id: &str) -> Option<&StarterTemplate> {
        self.templates.iter().find(|t| t.id == template_id)
    }

    pub fn get_by_category(
        &self,
        category: StarterTemplateCategory,
    ) -> Option<&StarterTemplate> {
        self.templates.iter().find(|t| t.category == category)
    }
}

/// Materializes a starter template into customer-scoped product state.
///
/// Per RFC 012, shipped system templates are immutable; materialization
/// creates customer-owned copies in tenant/workspace/project scope.
pub fn materialize_template(
    template: &StarterTemplate,
    tenant_id: &TenantId,
    workspace_id: &WorkspaceId,
    project_id: &ProjectId,
    now: u64,
) -> BootstrapProvenance {
    let materialized_assets = template
        .prompt_assets
        .iter()
        .map(|name| MaterializedAsset {
            asset_type: "prompt_asset".to_owned(),
            asset_id: format!("{}_{}", project_id.as_str(), name),
            source_template_ref: format!("{}:{}", template.id, name),
            diverged: false,
        })
        .chain(template.policy_presets.iter().map(|name| MaterializedAsset {
            asset_type: "policy_preset".to_owned(),
            asset_id: format!("{}_{}", project_id.as_str(), name),
            source_template_ref: format!("{}:{}", template.id, name),
            diverged: false,
        }))
        .collect();

    BootstrapProvenance {
        project_id: project_id.clone(),
        tenant_id: tenant_id.clone(),
        workspace_id: workspace_id.clone(),
        template_id: template.id.clone(),
        template_category: template.category,
        materialized_at: now,
        materialized_by: None,
        materialized_assets,
    }
}

/// Creates the canonical onboarding checklist for a project.
pub fn create_onboarding_checklist(
    project_id: &ProjectId,
    template_id: Option<&str>,
) -> OnboardingProgress {
    OnboardingProgress {
        project_id: project_id.clone(),
        template_id: template_id.map(|s| s.to_owned()),
        state: OnboardingFlowState::NotStarted,
        steps: vec![
            OnboardingStep {
                step_id: "create_project".to_owned(),
                name: "Create project".to_owned(),
                completed: false,
            },
            OnboardingStep {
                step_id: "select_template".to_owned(),
                name: "Choose starter template".to_owned(),
                completed: false,
            },
            OnboardingStep {
                step_id: "configure_provider".to_owned(),
                name: "Configure provider connection".to_owned(),
                completed: false,
            },
            OnboardingStep {
                step_id: "create_operator".to_owned(),
                name: "Create operator account".to_owned(),
                completed: false,
            },
            OnboardingStep {
                step_id: "import_assets".to_owned(),
                name: "Import prompts or knowledge".to_owned(),
                completed: false,
            },
            OnboardingStep {
                step_id: "first_run".to_owned(),
                name: "Run first workflow".to_owned(),
                completed: false,
            },
            OnboardingStep {
                step_id: "inspect_results".to_owned(),
                name: "Inspect results in control plane".to_owned(),
                completed: false,
            },
        ],
        started_at: None,
        completed_at: None,
    }
}

/// Canonical prompt import with reconciliation per RFC 012.
///
/// Matches by explicit import ID if present, otherwise by name + content_hash.
/// Idempotent: repeated import of same content is a no-op (Reused).
pub fn reconcile_prompt_import(
    existing_names: &std::collections::HashMap<String, String>, // name -> content_hash
    import_name: &str,
    import_content_hash: &str,
) -> ImportOutcome {
    match existing_names.get(import_name) {
        Some(existing_hash) if existing_hash == import_content_hash => ImportOutcome::Reused,
        Some(_) => ImportOutcome::Conflicted,
        None => ImportOutcome::Created,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn v1_registry_has_three_templates() {
        let registry = StarterTemplateRegistry::v1_defaults();
        assert_eq!(registry.list().len(), 3);
    }

    #[test]
    fn get_by_id() {
        let registry = StarterTemplateRegistry::v1_defaults();
        let t = registry.get("knowledge-assistant").unwrap();
        assert_eq!(t.category, StarterTemplateCategory::KnowledgeAssistant);
    }

    #[test]
    fn get_by_category() {
        let registry = StarterTemplateRegistry::v1_defaults();
        let t = registry
            .get_by_category(StarterTemplateCategory::ApprovalGatedWorker)
            .unwrap();
        assert_eq!(t.id, "approval-gated-worker");
    }

    #[test]
    fn materialize_creates_provenance() {
        let registry = StarterTemplateRegistry::v1_defaults();
        let template = registry.get("knowledge-assistant").unwrap();

        let provenance = materialize_template(
            template,
            &TenantId::new("t1"),
            &WorkspaceId::new("w1"),
            &ProjectId::new("p1"),
            1000,
        );

        assert_eq!(provenance.template_id, "knowledge-assistant");
        assert_eq!(
            provenance.template_category,
            StarterTemplateCategory::KnowledgeAssistant
        );
        // 2 prompt assets + 1 policy preset = 3 materialized assets
        assert_eq!(provenance.materialized_assets.len(), 3);
        assert!(provenance
            .materialized_assets
            .iter()
            .all(|a| !a.diverged));
    }

    #[test]
    fn onboarding_checklist_has_seven_steps() {
        let checklist = create_onboarding_checklist(&ProjectId::new("p1"), Some("test-template"));
        assert_eq!(checklist.steps.len(), 7);
        assert_eq!(checklist.state, OnboardingFlowState::NotStarted);
        assert!(checklist.steps.iter().all(|s| !s.completed));
    }

    #[test]
    fn reconcile_import_new_asset() {
        let existing = HashMap::new();
        assert_eq!(
            reconcile_prompt_import(&existing, "new.prompt", "hash1"),
            ImportOutcome::Created
        );
    }

    #[test]
    fn reconcile_import_same_content_is_reused() {
        let mut existing = HashMap::new();
        existing.insert("my.prompt".to_owned(), "hash1".to_owned());
        assert_eq!(
            reconcile_prompt_import(&existing, "my.prompt", "hash1"),
            ImportOutcome::Reused
        );
    }

    #[test]
    fn reconcile_import_different_content_is_conflicted() {
        let mut existing = HashMap::new();
        existing.insert("my.prompt".to_owned(), "hash1".to_owned());
        assert_eq!(
            reconcile_prompt_import(&existing, "my.prompt", "hash2"),
            ImportOutcome::Conflicted
        );
    }

    /// RFC 012 §4: checklist MUST contain every canonical bootstrap step, in order.
    ///
    /// Each step_id is load-bearing — the frontend tracks progress by ID,
    /// so removing or renaming any step is a breaking change.
    #[test]
    fn onboarding_checklist_contains_all_required_step_ids_in_order() {
        const REQUIRED_STEPS: &[&str] = &[
            "create_project",
            "select_template",
            "configure_provider",
            "create_operator",
            "import_assets",
            "first_run",
            "inspect_results",
        ];

        let checklist = create_onboarding_checklist(
            &ProjectId::new("p1"),
            Some("knowledge-assistant"),
        );
        let step_ids: Vec<&str> = checklist.steps.iter().map(|s| s.step_id.as_str()).collect();

        // Every required step must be present.
        for required in REQUIRED_STEPS {
            assert!(
                step_ids.contains(required),
                "RFC 012 requires step '{}'; present: {:?}", required, step_ids
            );
        }

        // Required steps must appear in canonical order.
        let positions: Vec<usize> = REQUIRED_STEPS
            .iter()
            .map(|req| step_ids.iter().position(|id| id == req).unwrap())
            .collect();
        for i in 1..positions.len() {
            assert!(
                positions[i - 1] < positions[i],
                "RFC 012: '{}' must appear before '{}' in the checklist",
                REQUIRED_STEPS[i - 1],
                REQUIRED_STEPS[i],
            );
        }
    }
}

// ── RFC 012 Gap Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod rfc012_tests {
    use super::*;
    use std::collections::HashMap;

    /// RFC 012: three mandatory starter template categories must all be present.
    #[test]
    fn rfc012_mandatory_three_starter_templates_present() {
        let registry = StarterTemplateRegistry::v1_defaults();
        let templates = registry.list();
        assert!(templates.len() >= 3,
            "RFC 012: v1 must ship at least 3 starter templates");

        let has_ka = templates.iter().any(|t| t.category == StarterTemplateCategory::KnowledgeAssistant);
        let has_agw = templates.iter().any(|t| t.category == StarterTemplateCategory::ApprovalGatedWorker);
        let has_msw = templates.iter().any(|t| t.category == StarterTemplateCategory::MultiStepWorkflow);

        assert!(has_ka,  "RFC 012: KnowledgeAssistant template required");
        assert!(has_agw, "RFC 012: ApprovalGatedWorker template required");
        assert!(has_msw, "RFC 012: MultiStepWorkflow template required");
    }

    /// RFC 012: bootstrap provenance must record starter template origin.
    #[test]
    fn rfc012_bootstrap_records_template_provenance() {
        let registry = StarterTemplateRegistry::v1_defaults();
        let template = registry.get("knowledge-assistant").unwrap();
        let provenance = materialize_template(
            template,
            &cairn_domain::TenantId::new("t1"),
            &cairn_domain::WorkspaceId::new("w1"),
            &cairn_domain::ProjectId::new("p1"),
            12345,
        );

        assert_eq!(provenance.template_id, "knowledge-assistant",
            "RFC 012: provenance must record which template was selected");
        assert!(provenance.materialized_at == 12345,
            "RFC 012: provenance must record when materialization happened");
        assert!(!provenance.materialized_assets.is_empty(),
            "RFC 012: provenance must record which assets were materialized");
    }

    /// RFC 012: bootstrap must be idempotent — repeated import of same content = Reused.
    #[test]
    fn rfc012_prompt_import_is_idempotent() {
        let mut existing = HashMap::new();
        existing.insert("system.prompt".to_owned(), "abc123".to_owned());

        // First import = Created.
        assert_eq!(
            reconcile_prompt_import(&HashMap::new(), "system.prompt", "abc123"),
            ImportOutcome::Created,
            "RFC 012: first import of new prompt must be Created"
        );

        // Second import with same content = Reused (idempotent).
        assert_eq!(
            reconcile_prompt_import(&existing, "system.prompt", "abc123"),
            ImportOutcome::Reused,
            "RFC 012: repeat import of same content must be Reused (idempotent)"
        );
    }

    /// RFC 012: prompt import must NEVER silently mutate an existing version.
    /// Changed content must produce Conflicted, not silent overwrite.
    #[test]
    fn rfc012_changed_content_produces_conflict_not_silent_overwrite() {
        let mut existing = HashMap::new();
        existing.insert("agent.prompt".to_owned(), "original_hash".to_owned());

        let outcome = reconcile_prompt_import(&existing, "agent.prompt", "new_hash");
        assert_eq!(outcome, ImportOutcome::Conflicted,
            "RFC 012: changed content must produce Conflicted, not silent overwrite");
        assert_ne!(outcome, ImportOutcome::Reused,
            "RFC 012: must not silently reuse when content changed");
    }

    /// RFC 012: materialized assets must not diverge by default.
    #[test]
    fn rfc012_materialized_assets_are_not_diverged_on_creation() {
        let registry = StarterTemplateRegistry::v1_defaults();
        let template = registry.get("approval-gated-worker").unwrap();
        let provenance = materialize_template(
            template,
            &cairn_domain::TenantId::new("t1"),
            &cairn_domain::WorkspaceId::new("w1"),
            &cairn_domain::ProjectId::new("p1"),
            100,
        );
        // RFC 012: newly materialized assets must not show as diverged from shipped defaults.
        for asset in &provenance.materialized_assets {
            assert!(!asset.diverged,
                "RFC 012: freshly materialized asset '{}' must not be marked diverged",
                asset.asset_id);
        }
    }
}
