//! RFC 012 — onboarding flow end-to-end integration tests.
//!
//! Tests the bootstrap arc operators follow when first deploying Cairn:
//!   1. List available starter templates from the V1 registry
//!   2. Select and materialize a template — verify provenance records
//!      the tenant/workspace/project scope and the materialized assets
//!   3. Create the canonical onboarding checklist for the project
//!   4. Compute progress — all steps must be incomplete initially
//!   5. Mark a step complete via direct mutation
//!   6. Compute progress again — that step must show completed=true

use std::sync::Arc;

use cairn_api::onboarding::{
    compute_progress, create_onboarding_checklist, materialize_template,
    StarterTemplateRegistry,
};
use cairn_domain::onboarding::{
    OnboardingFlowState, StarterTemplateCategory,
};
use cairn_domain::{ProjectId, TenantId, WorkspaceId};
use cairn_runtime::services::{
    ProjectServiceImpl, TenantServiceImpl, WorkspaceServiceImpl,
};
use cairn_runtime::projects::ProjectService;
use cairn_runtime::tenants::TenantService;
use cairn_runtime::workspaces::WorkspaceService;
use cairn_store::InMemoryStore;
use cairn_domain::ProjectKey;

/// (1) List available templates — V1 registry must contain exactly 3.
#[test]
fn list_available_templates() {
    let registry = StarterTemplateRegistry::v1_defaults();
    let templates = registry.list();

    assert_eq!(templates.len(), 3, "V1 registry must ship exactly 3 starter templates");

    let ids: Vec<&str> = templates.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&"knowledge-assistant"));
    assert!(ids.contains(&"approval-gated-worker"));
    assert!(ids.contains(&"multi-step-workflow"));

    // All three required categories must be present.
    assert!(templates.iter().any(|t| t.category == StarterTemplateCategory::KnowledgeAssistant));
    assert!(templates.iter().any(|t| t.category == StarterTemplateCategory::ApprovalGatedWorker));
    assert!(templates.iter().any(|t| t.category == StarterTemplateCategory::MultiStepWorkflow));

    // Each template must have a name, description, and at least one asset.
    for t in templates {
        assert!(!t.name.is_empty(), "template '{}' must have a name", t.id);
        assert!(!t.description.is_empty(), "template '{}' must have a description", t.id);
        assert!(
            !t.prompt_assets.is_empty() || !t.policy_presets.is_empty(),
            "template '{}' must have at least one prompt asset or policy preset", t.id
        );
    }
}

/// get() and get_by_category() must return the correct template.
#[test]
fn registry_lookup_by_id_and_category() {
    let registry = StarterTemplateRegistry::v1_defaults();

    let by_id = registry.get("knowledge-assistant").unwrap();
    assert_eq!(by_id.category, StarterTemplateCategory::KnowledgeAssistant);

    let by_cat = registry.get_by_category(StarterTemplateCategory::ApprovalGatedWorker).unwrap();
    assert_eq!(by_cat.id, "approval-gated-worker");

    assert!(registry.get("nonexistent").is_none());
}

/// (2) Materialize a template — verify tenant/workspace/project scope in
/// the provenance record and that all assets are scoped correctly.
#[tokio::test]
async fn materialize_template_creates_provenance_with_correct_scope() {
    let store = Arc::new(InMemoryStore::new());

    // Create tenant / workspace / project via actual runtime services.
    let tenant_id = TenantId::new("t_onboard");
    let workspace_id = WorkspaceId::new("w_onboard");
    let project_id = ProjectId::new("p_onboard");
    let project_key = ProjectKey::new("t_onboard", "w_onboard", "p_onboard");

    TenantServiceImpl::new(store.clone())
        .create(tenant_id.clone(), "Onboard Co.".to_owned())
        .await
        .unwrap();
    WorkspaceServiceImpl::new(store.clone())
        .create(tenant_id.clone(), workspace_id.clone(), "Main WS".to_owned())
        .await
        .unwrap();
    ProjectServiceImpl::new(store.clone())
        .create(project_key.clone(), "Onboard Project".to_owned())
        .await
        .unwrap();

    // Select the knowledge-assistant template and materialize it.
    let registry = StarterTemplateRegistry::v1_defaults();
    let template = registry.get("knowledge-assistant").unwrap();

    let now = 1_700_000_000_000u64;
    let provenance = materialize_template(template, &tenant_id, &workspace_id, &project_id, now);

    // Verify the provenance records the correct scope.
    assert_eq!(provenance.tenant_id, tenant_id);
    assert_eq!(provenance.workspace_id, workspace_id);
    assert_eq!(provenance.project_id, project_id);
    assert_eq!(provenance.template_id, "knowledge-assistant");
    assert_eq!(provenance.template_category, StarterTemplateCategory::KnowledgeAssistant);
    assert_eq!(provenance.materialized_at, now);
    assert!(provenance.materialized_by.is_none());

    // Assets must be scoped to the project and not diverged.
    assert!(
        !provenance.materialized_assets.is_empty(),
        "provenance must record at least one materialized asset"
    );
    for asset in &provenance.materialized_assets {
        assert!(
            asset.asset_id.starts_with("p_onboard_"),
            "asset_id must be scoped to the project; got: {}", asset.asset_id
        );
        assert!(!asset.diverged, "freshly materialized assets must not be marked diverged");
        assert!(
            asset.source_template_ref.starts_with("knowledge-assistant:"),
            "source_template_ref must reference the template"
        );
    }

    // 2 prompt assets + 1 policy preset = 3 materialized assets.
    assert_eq!(
        provenance.materialized_assets.len(), 3,
        "knowledge-assistant has 2 prompt_assets + 1 policy_preset = 3 total"
    );
}

/// (3+4) Create the checklist and verify all steps are incomplete initially.
#[test]
fn create_checklist_all_steps_initially_incomplete() {
    let project_id = ProjectId::new("p_checklist");

    // (3) Create the canonical checklist.
    let checklist = create_onboarding_checklist(&project_id, Some("knowledge-assistant"));

    assert_eq!(checklist.project_id, project_id);
    assert_eq!(checklist.template_id.as_deref(), Some("knowledge-assistant"));
    assert_eq!(checklist.state, OnboardingFlowState::NotStarted);
    assert_eq!(checklist.steps.len(), 7, "V1 checklist must have exactly 7 steps");
    assert!(checklist.started_at.is_none());
    assert!(checklist.completed_at.is_none());

    // (4) Compute progress — all steps must be incomplete.
    let progress = compute_progress(&checklist);

    assert_eq!(progress.len(), 7, "progress must have one entry per checklist step");

    for step in &progress {
        assert!(
            !step.completed,
            "step '{}' must be incomplete on a fresh checklist", step.step_id
        );
        assert!(
            step.completed_at.is_none(),
            "step '{}' must have no completed_at on a fresh checklist", step.step_id
        );
        assert!(!step.label.is_empty(), "step '{}' must have a non-empty label", step.step_id);
        assert!(!step.step_id.is_empty());
    }

    // Required steps must be flagged.
    let required_ids = ["create_project", "configure_provider", "create_operator", "first_run"];
    for p in &progress {
        let expected = required_ids.contains(&p.step_id.as_str());
        assert_eq!(
            p.required, expected,
            "step '{}' required flag mismatch", p.step_id
        );
    }
}

/// (5+6) Mark a step complete via direct mutation, then verify progress reflects it.
#[test]
fn mark_step_complete_reflects_in_progress() {
    let project_id = ProjectId::new("p_step_done");
    let mut checklist = create_onboarding_checklist(&project_id, None);

    // Pre-condition: create_project is incomplete.
    let before = compute_progress(&checklist);
    let create_before = before.iter().find(|p| p.step_id == "create_project").unwrap();
    assert!(!create_before.completed, "create_project must start incomplete");

    // (5) Mark create_project as complete.
    let step = checklist.steps.iter_mut().find(|s| s.step_id == "create_project").unwrap();
    step.completed = true;

    // (6) Compute progress — create_project must now show completed=true.
    let after = compute_progress(&checklist);
    let create_after = after.iter().find(|p| p.step_id == "create_project").unwrap();

    assert!(
        create_after.completed,
        "create_project must show completed=true after being marked done"
    );

    // All other steps must remain incomplete.
    let others_incomplete = after.iter()
        .filter(|p| p.step_id != "create_project")
        .all(|p| !p.completed);
    assert!(
        others_incomplete,
        "marking one step complete must not affect any other step"
    );

    // Completing a second step.
    let step2 = checklist.steps.iter_mut().find(|s| s.step_id == "configure_provider").unwrap();
    step2.completed = true;

    let after2 = compute_progress(&checklist);
    let completed_count = after2.iter().filter(|p| p.completed).count();
    assert_eq!(completed_count, 2, "exactly 2 steps must be marked complete");

    let configure_after = after2.iter().find(|p| p.step_id == "configure_provider").unwrap();
    assert!(configure_after.completed);
    assert!(configure_after.required, "configure_provider is a required step");
}

/// compute_progress with a fully completed checklist (all steps done).
#[test]
fn fully_completed_checklist_all_steps_done() {
    let project_id = ProjectId::new("p_all_done");
    let mut checklist = create_onboarding_checklist(&project_id, Some("multi-step-workflow"));
    let now_ms = 1_700_000_000_000u64;
    checklist.completed_at = Some(now_ms);

    for step in &mut checklist.steps {
        step.completed = true;
    }

    let progress = compute_progress(&checklist);

    assert_eq!(progress.len(), 7);
    for p in &progress {
        assert!(p.completed, "all steps must be completed");
        assert_eq!(
            p.completed_at,
            Some(now_ms),
            "completed_at must propagate from the checklist"
        );
    }
}

/// All three templates materialize the correct number of assets.
#[test]
fn all_templates_materialize_expected_asset_counts() {
    let registry = StarterTemplateRegistry::v1_defaults();
    let tenant_id = TenantId::new("t_assets");
    let workspace_id = WorkspaceId::new("w_assets");
    let now = 0u64;

    let expected = [
        ("knowledge-assistant",   3usize), // 2 prompt + 1 policy
        ("approval-gated-worker", 2usize), // 1 prompt + 1 policy
        ("multi-step-workflow",   3usize), // 2 prompt + 1 policy
    ];

    for (template_id, expected_count) in expected {
        let template = registry.get(template_id).unwrap();
        let project_id = ProjectId::new(format!("proj_{template_id}"));
        let provenance = materialize_template(template, &tenant_id, &workspace_id, &project_id, now);
        assert_eq!(
            provenance.materialized_assets.len(),
            expected_count,
            "template '{}' must materialize {} assets", template_id, expected_count
        );
    }
}

/// reconcile_prompt_imports batch function produces correct counts.
#[test]
fn batch_import_reconciliation_counts() {
    use cairn_api::onboarding::{PromptImportItem, reconcile_prompt_imports};
    use std::collections::HashMap;

    let mut existing = HashMap::new();
    existing.insert("system.prompt".to_owned(), "hash_v1".to_owned());

    let items = vec![
        // New asset.
        PromptImportItem { name: "new.prompt".into(), content_hash: "hash_new".into(), import_id: None },
        // Same hash — reused.
        PromptImportItem { name: "system.prompt".into(), content_hash: "hash_v1".into(), import_id: None },
        // Changed hash — updated.
        PromptImportItem { name: "system.prompt".into(), content_hash: "hash_v2".into(), import_id: Some("imp_sys".into()) },
    ];

    // For the last item: import_id="imp_sys" not in existing → Created.
    let report = reconcile_prompt_imports(&existing, &items);
    assert_eq!(report.created_count, 2, "two new keys: 'new.prompt' and 'imp_sys'");
    assert_eq!(report.skipped_count, 1, "system.prompt with hash_v1 is reused");
    assert_eq!(report.updated_count, 0);
    assert_eq!(report.total(), 3);
}
