//! End-to-end integration tests exercising the full agent lifecycle
//! across multiple services.
//!
//! Each test spins up an in-memory AppState with real runtime services.

#![cfg(feature = "in-memory-runtime")]

use std::sync::Arc;

use cairn_domain::{
    ApprovalDecision, ApprovalId, ApprovalRequirement, EventEnvelope, EventSource, ProjectKey,
    RunId, RunState, RunStateChanged, RuntimeEvent, SessionId, StateTransition, TaskId, TaskState,
    TenantId, WorkspaceId,
};
use cairn_runtime::{
    ApprovalService, InMemoryServices, ProjectService, QuotaService, TenantService,
    WorkspaceService,
};
use cairn_store::projections::{RunReadModel, SessionReadModel, TaskReadModel};
use cairn_store::EventLog;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Transition a run from Pending to Running by emitting a state-change event.
async fn activate_run(svc: &InMemoryServices, project: &ProjectKey, run_id: &RunId) {
    let event = EventEnvelope::for_runtime_event(
        cairn_domain::EventId::new(format!("evt_activate_{}", run_id.as_str())),
        EventSource::Runtime,
        RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: Some(RunState::Pending),
                to: RunState::Running,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }),
    );
    svc.store.append(&[event]).await.unwrap();
}

/// Create a fully wired runtime with a tenant, workspace, and project.
async fn setup() -> (Arc<InMemoryServices>, ProjectKey) {
    let svc = Arc::new(InMemoryServices::new());
    let tenant_id = TenantId::new("e2e_tenant");
    let workspace_id = WorkspaceId::new("e2e_ws");
    let project = ProjectKey::new("e2e_tenant", "e2e_ws", "e2e_proj");

    svc.tenants
        .create(tenant_id.clone(), "E2E Tenant".to_owned())
        .await
        .unwrap();
    svc.workspaces
        .create(tenant_id, workspace_id, "E2E Workspace".to_owned())
        .await
        .unwrap();
    svc.projects
        .create(project.clone(), "E2E Project".to_owned())
        .await
        .unwrap();

    (svc, project)
}

// ══════════════════════════════════════════════════════════════════════════════
// Test 1: Full agent session lifecycle
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_full_session_lifecycle() {
    let (svc, project) = setup().await;

    // ── 1. Create session ────────────────────────────────────────────────
    let session = svc
        .sessions
        .create(&project, SessionId::new("sess_e2e_1"))
        .await
        .unwrap();
    assert_eq!(session.session_id.as_str(), "sess_e2e_1");

    // Verify read model.
    let fetched = SessionReadModel::get(svc.store.as_ref(), &SessionId::new("sess_e2e_1"))
        .await
        .unwrap();
    assert!(fetched.is_some());

    // ── 2. Start run (creates in Pending, then activate to Running) ────
    let run = svc
        .runs
        .start(
            &project,
            &SessionId::new("sess_e2e_1"),
            RunId::new("run_e2e_1"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Pending);

    // Activate: Pending → Running.
    activate_run(&svc, &project, &RunId::new("run_e2e_1")).await;

    // Verify read model shows Running.
    let run_record = RunReadModel::get(svc.store.as_ref(), &RunId::new("run_e2e_1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run_record.state, RunState::Running);

    // ── 3. Submit tasks ──────────────────────────────────────────────────
    let task1 = svc
        .tasks
        .submit(
            &project,
            TaskId::new("task_e2e_1"),
            Some(RunId::new("run_e2e_1")),
            None,
            1,
        )
        .await
        .unwrap();
    assert_eq!(task1.state, TaskState::Queued);

    let task2 = svc
        .tasks
        .submit(
            &project,
            TaskId::new("task_e2e_2"),
            Some(RunId::new("run_e2e_1")),
            None,
            2,
        )
        .await
        .unwrap();
    assert_eq!(task2.state, TaskState::Queued);

    // ── 4. Claim tasks ───────────────────────────────────────────────────
    let claimed = svc
        .tasks
        .claim(&TaskId::new("task_e2e_1"), "worker_1".to_owned(), 30_000)
        .await
        .unwrap();
    assert_eq!(claimed.state, TaskState::Leased);

    // ── 5. Start and complete tasks ──────────────────────────────────────
    svc.tasks.start(&TaskId::new("task_e2e_1")).await.unwrap();
    let completed = svc
        .tasks
        .complete(&TaskId::new("task_e2e_1"))
        .await
        .unwrap();
    assert!(completed.state.is_terminal());

    // Claim + start + complete task 2.
    svc.tasks
        .claim(&TaskId::new("task_e2e_2"), "worker_1".to_owned(), 30_000)
        .await
        .unwrap();
    svc.tasks.start(&TaskId::new("task_e2e_2")).await.unwrap();
    svc.tasks
        .complete(&TaskId::new("task_e2e_2"))
        .await
        .unwrap();

    // ── 6. Complete run ──────────────────────────────────────────────────
    let run_done = svc.runs.complete(&RunId::new("run_e2e_1")).await.unwrap();
    assert_eq!(run_done.state, RunState::Completed);

    // ── 7. Verify events emitted ─────────────────────────────────────────
    let events = svc.store.read_stream(None, 100).await.unwrap();
    // Should have: session created, run created, 2x task created,
    // task state changes (claim, start, complete x2), run state change.
    assert!(
        events.len() >= 8,
        "expected >= 8 events, got {}",
        events.len()
    );

    // ── 8. Verify final read model state ─────────────────────────────────
    let final_run = RunReadModel::get(svc.store.as_ref(), &RunId::new("run_e2e_1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_run.state, RunState::Completed);

    let final_task = TaskReadModel::get(svc.store.as_ref(), &TaskId::new("task_e2e_1"))
        .await
        .unwrap()
        .unwrap();
    assert!(final_task.state.is_terminal());
}

// ══════════════════════════════════════════════════════════════════════════════
// Test 2: Approval workflow
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_approval_workflow() {
    let (svc, project) = setup().await;

    // Start a session and run, then activate.
    svc.sessions
        .create(&project, SessionId::new("sess_apr"))
        .await
        .unwrap();
    svc.runs
        .start(
            &project,
            &SessionId::new("sess_apr"),
            RunId::new("run_apr"),
            None,
        )
        .await
        .unwrap();
    activate_run(&svc, &project, &RunId::new("run_apr")).await;

    // ── 1. Request approval ──────────────────────────────────────────────
    let approval = svc
        .approvals
        .request(
            &project,
            ApprovalId::new("apr_e2e_1"),
            Some(RunId::new("run_apr")),
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();
    assert!(approval.decision.is_none(), "approval should be pending");

    // ── 2. Verify pending ────────────────────────────────────────────────
    let pending = svc.approvals.list_pending(&project, 10, 0).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].approval_id.as_str(), "apr_e2e_1");

    // ── 3. Approve ───────────────────────────────────────────────────────
    let resolved = svc
        .approvals
        .resolve(&ApprovalId::new("apr_e2e_1"), ApprovalDecision::Approved)
        .await
        .unwrap();
    assert_eq!(resolved.decision, Some(ApprovalDecision::Approved));

    // ── 4. Verify resolved ───────────────────────────────────────────────
    let pending_after = svc.approvals.list_pending(&project, 10, 0).await.unwrap();
    assert!(
        pending_after.is_empty(),
        "no pending approvals after resolution"
    );

    // The run should still be Running (approval doesn't auto-complete the run).
    let run = RunReadModel::get(svc.store.as_ref(), &RunId::new("run_apr"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.state, RunState::Running);
}

// ══════════════════════════════════════════════════════════════════════════════
// Test 3: Provider fallback
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_provider_fallback() {
    use cairn_domain::providers::{
        GenerationProvider, GenerationResponse, OperationKind, ProviderAdapterError,
        ProviderBindingSettings,
    };
    use cairn_domain::ProviderModelId;
    use cairn_runtime::{ProviderHealthTracker, ProviderRouter, RoutableProvider, RoutingConfig};

    /// Mock provider that always fails.
    struct FailingProvider;
    #[async_trait::async_trait]
    impl GenerationProvider for FailingProvider {
        async fn generate(
            &self,
            _model_id: &str,
            _messages: Vec<serde_json::Value>,
            _settings: &ProviderBindingSettings,
            _tools: &[serde_json::Value],
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            Err(ProviderAdapterError::TransportFailure(
                "primary down".into(),
            ))
        }
    }

    /// Mock provider that always succeeds.
    struct SuccessProvider;
    #[async_trait::async_trait]
    impl GenerationProvider for SuccessProvider {
        async fn generate(
            &self,
            model_id: &str,
            _messages: Vec<serde_json::Value>,
            _settings: &ProviderBindingSettings,
            _tools: &[serde_json::Value],
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            Ok(GenerationResponse {
                text: "fallback response".into(),
                input_tokens: Some(10),
                output_tokens: Some(5),
                model_id: model_id.to_owned(),
                tool_calls: vec![],
                finish_reason: None,
            })
        }
    }

    let health = Arc::new(ProviderHealthTracker::new());
    let mut router = ProviderRouter::new(RoutingConfig::default(), health.clone());

    // Register primary (failing) and fallback (succeeding).
    router.register(
        cairn_domain::ProviderConnectionId::new("primary_conn"),
        Arc::new(FailingProvider),
    );
    router.register(
        cairn_domain::ProviderConnectionId::new("fallback_conn"),
        Arc::new(SuccessProvider),
    );

    let project = ProjectKey::new("t", "w", "p");

    fn binding(id: &str, conn: &str) -> cairn_domain::providers::ProviderBindingRecord {
        cairn_domain::providers::ProviderBindingRecord {
            provider_binding_id: cairn_domain::ProviderBindingId::new(id),
            project: ProjectKey::new("t", "w", "p"),
            provider_connection_id: cairn_domain::ProviderConnectionId::new(conn),
            provider_model_id: ProviderModelId::new("model-1"),
            operation_kind: OperationKind::Generate,
            settings: ProviderBindingSettings::default(),
            active: true,
            created_at: 1000,
        }
    }

    let candidates = vec![
        RoutableProvider::new(binding("bind_primary", "primary_conn"), vec![]),
        RoutableProvider::new(binding("bind_fallback", "fallback_conn"), vec![]),
    ];

    // ── Route — primary should fail, fallback should succeed ─────────────
    let outcome = router
        .route(
            &project,
            OperationKind::Generate,
            &cairn_domain::selectors::SelectorContext::default(),
            candidates,
            "model-1",
            vec![],
            &ProviderBindingSettings::default(),
        )
        .await;

    assert_eq!(
        outcome.decision.final_status,
        cairn_domain::providers::RouteDecisionStatus::Selected
    );
    assert!(
        outcome.decision.fallback_used,
        "fallback must have been used"
    );
    assert_eq!(
        outcome.decision.selected_provider_binding_id,
        Some(cairn_domain::ProviderBindingId::new("bind_fallback"))
    );
    assert!(outcome.response.is_some());
    assert_eq!(outcome.response.unwrap().text, "fallback response");

    // ── Verify health tracker recorded the failure and success ───────────
    let primary_health = health
        .get(&cairn_domain::ProviderConnectionId::new("primary_conn"))
        .unwrap();
    assert_eq!(primary_health.failure_count, 1);
    assert_eq!(primary_health.success_count, 0);

    let fallback_health = health
        .get(&cairn_domain::ProviderConnectionId::new("fallback_conn"))
        .unwrap();
    assert_eq!(fallback_health.success_count, 1);
    assert_eq!(fallback_health.failure_count, 0);
}

// ══════════════════════════════════════════════════════════════════════════════
// Test 4: Entitlement enforcement
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_entitlement_enforcement() {
    use cairn_domain::WorkspaceId;
    use cairn_runtime::WorkspaceQuotaManager;

    let (svc, project) = setup().await;

    // ── Set workspace quota: max 2 concurrent runs ───────────────────────
    let mgr = WorkspaceQuotaManager::new();
    mgr.set_policy(cairn_runtime::WorkspaceQuotaPolicy {
        workspace_id: WorkspaceId::new("e2e_ws"),
        tenant_id: TenantId::new("e2e_tenant"),
        max_runs_per_hour: 100,
        max_concurrent_runs: 2,
        max_storage_mb: 1000,
        max_tokens_per_day: 1_000_000,
    });

    // Also set the tenant-level quota via the runtime service.
    svc.quotas
        .set_quota(TenantId::new("e2e_tenant"), 2, 100, 100)
        .await
        .unwrap();

    // ── Create sessions and start runs up to the limit ───────────────────
    for i in 1..=2 {
        let sess_id = SessionId::new(format!("sess_ent_{i}"));
        let run_id = RunId::new(format!("run_ent_{i}"));
        svc.sessions
            .create(&project, sess_id.clone())
            .await
            .unwrap();
        svc.runs
            .start(&project, &sess_id, run_id.clone(), None)
            .await
            .unwrap();
        activate_run(&svc, &project, &run_id).await;
    }

    // ── Verify workspace quota enforcement ───────────────────────────────
    // Record the runs in the workspace manager.
    mgr.record_run_started("e2e_ws");
    mgr.record_run_started("e2e_ws");
    let quota_result = mgr.check_run_quota(&WorkspaceId::new("e2e_ws"));
    assert!(
        quota_result.is_err(),
        "third run should be rejected by workspace quota"
    );

    // ── Verify tenant quota enforcement ──────────────────────────────────
    let sess3 = SessionId::new("sess_ent_3");
    svc.sessions.create(&project, sess3.clone()).await.unwrap();
    let run_err = svc
        .runs
        .start(&project, &sess3, RunId::new("run_ent_3"), None)
        .await;
    assert!(
        run_err.is_err(),
        "third run should be rejected by tenant quota"
    );

    // ── Complete one run and verify quota is available again ──────────────
    svc.runs.complete(&RunId::new("run_ent_1")).await.unwrap();
    mgr.record_run_completed("e2e_ws");

    let quota_after = mgr.check_run_quota(&WorkspaceId::new("e2e_ws"));
    assert!(
        quota_after.is_ok(),
        "quota should be available after completing a run"
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Test 5: Template application
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_template_application() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use cairn_api::auth::AuthPrincipal;
    use cairn_api::bootstrap::BootstrapConfig;
    use cairn_app::AppBootstrap;
    use cairn_domain::tenancy::TenantKey;
    use cairn_domain::OperatorId;
    use tower::ServiceExt;

    const TOKEN: &str = "e2e-template-token";

    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    // ── 1. List templates ────────────────────────────────────────────────
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/onboarding/templates")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let templates: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(
        !templates.is_empty(),
        "should have at least one starter template"
    );

    // Find a template ID to apply.
    let template_id = templates[0]["id"]
        .as_str()
        .expect("template should have an id field");

    // ── 2. Apply template to project ─────────────────────────────────────
    let apply_body = serde_json::json!({
        "template_id": template_id,
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/onboarding/template")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(apply_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.status().is_success(),
        "apply template should succeed, got {}",
        resp.status()
    );
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // ── 3. Verify template was applied ───────────────────────────────────
    // The response should contain the template information or materialized artifacts.
    assert!(
        result.is_object(),
        "apply response should be a JSON object: {result}"
    );

    // ── 4. List templates again — should still be available ──────────────
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/onboarding/templates")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let templates_after: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        templates.len(),
        templates_after.len(),
        "applying a template should not remove it from the list"
    );
}
