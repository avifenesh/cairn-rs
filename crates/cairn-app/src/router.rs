//! HTTP router and application bootstrap.
//!
//! Contains `AppBootstrap` — the main entry point that constructs the
//! Axum router from the preserved route catalog, applies middleware,
//! and serves HTTP/HTTPS.

use async_trait::async_trait;
use axum::{
    extract::DefaultBodyLimit,
    http::{header, StatusCode},
    middleware::{from_fn, from_fn_with_state},
    response::IntoResponse,
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use axum_server::{tls_rustls::RustlsConfig, Handle as AxumServerHandle};
use cairn_api::auth::ServiceTokenRegistry;
use cairn_api::bootstrap::{BootstrapConfig, DeploymentMode, ServerBootstrap};
use cairn_api::http::{preserved_route_catalog, ApiError, HttpMethod};
use cairn_api::onboarding::ProviderBindingBootstrapService;
use cairn_domain::ProviderBindingRecord;
use cairn_graph::in_memory::InMemoryGraphStore;
use cairn_memory::bundles::BundleEnvelope;
use cairn_runtime::{
    InMemoryServices, ProviderBindingService, ProviderConnectionConfig, ProviderConnectionService,
};
use std::{future::Future, net::SocketAddr, sync::Arc};
use tokio::{net::TcpListener, runtime::Builder};
use tower_http::cors::{Any, CorsLayer};
use utoipa::{OpenApi, ToSchema};

use crate::bootstrap::shutdown_signal;
use crate::marketplace_routes;
use crate::repo_routes;
use crate::state::AppState;
use crate::telemetry_routes;
use crate::trigger_routes;
#[allow(unused_imports)]
use crate::*;

// ── OpenAPI documentation types ─────────────────────────────────────────────

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct SessionRecordDoc {
    session_id: String,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    state: String,
    created_at: u64,
    updated_at: u64,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct SessionListResponseDoc {
    items: Vec<SessionRecordDoc>,
    has_more: bool,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct RunRecordDoc {
    run_id: String,
    session_id: String,
    parent_run_id: Option<String>,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    state: String,
    created_at: u64,
    updated_at: u64,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct RunListResponseDoc {
    items: Vec<RunRecordDoc>,
    has_more: bool,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct TaskRecordDoc {
    task_id: String,
    parent_run_id: Option<String>,
    parent_task_id: Option<String>,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    state: String,
    created_at: u64,
    updated_at: u64,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct TenantRecordDoc {
    tenant_id: String,
    name: String,
    created_at: u64,
    updated_at: u64,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct WorkspaceRecordDoc {
    workspace_id: String,
    tenant_id: String,
    name: String,
    created_at: u64,
    updated_at: u64,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct ProjectRecordDoc {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    name: String,
    created_at: u64,
    updated_at: u64,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct ProviderConnectionRecordDoc {
    tenant_id: String,
    provider_connection_id: String,
    provider_family: String,
    adapter_type: String,
    status: String,
    registered_at: u64,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct ProviderConnectionListResponseDoc {
    items: Vec<ProviderConnectionRecordDoc>,
    has_more: bool,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct ProviderBindingRecordDoc {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    provider_binding_id: String,
    provider_connection_id: String,
    operation_kind: String,
    provider_model_id: String,
    active: bool,
}

#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub(crate) struct ProviderBindingListResponseDoc {
    items: Vec<ProviderBindingRecordDoc>,
    has_more: bool,
}

// ── Provider bootstrap adapter ──────────────────────────────────────────────

#[allow(dead_code)]
struct AppProviderBootstrap<'a> {
    provider_connections: &'a dyn ProviderConnectionService,
    provider_bindings: &'a dyn ProviderBindingService,
}

#[async_trait]
impl ProviderBindingBootstrapService for AppProviderBootstrap<'_> {
    async fn create_default_binding(
        &self,
        binding: ProviderBindingRecord,
    ) -> Result<ProviderBindingRecord, String> {
        if self
            .provider_connections
            .get(&binding.provider_connection_id)
            .await
            .map_err(|e| e.to_string())?
            .is_none()
        {
            self.provider_connections
                .create(
                    binding.project.tenant_id.clone(),
                    binding.provider_connection_id.clone(),
                    ProviderConnectionConfig {
                        provider_family: "openai".to_owned(),
                        adapter_type: "responses_api".to_owned(),
                        supported_models: vec![],
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
        }

        self.provider_bindings
            .create(
                binding.project,
                binding.provider_connection_id,
                binding.operation_kind,
                binding.provider_model_id,
                None,
            )
            .await
            .map_err(|e| e.to_string())
    }
}

// ── OpenAPI spec ────────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    paths(
        health_handler,
        list_sessions_handler,
        create_session_handler,
        list_runs_handler,
        create_run_handler,
        create_task_handler,
        create_tenant_handler,
        create_workspace_handler,
        delete_workspace_handler,
        create_project_handler,
        create_provider_connection_handler,
        list_provider_bindings_handler
    ),
    components(schemas(
        ApiError,
        HealthCheck,
        HealthReport,
        CreateSessionRequest,
        CreateRunRequest,
        CreateTaskRequest,
        CreateTenantRequest,
        CreateWorkspaceRequest,
        CreateProjectRequest,
        CreateProviderConnectionRequest,
        CreateProviderBindingRequest,
        StoreCredentialRequest,
        CredentialSummary,
        SessionRecordDoc,
        SessionListResponseDoc,
        RunRecordDoc,
        RunListResponseDoc,
        TaskRecordDoc,
        TenantRecordDoc,
        WorkspaceRecordDoc,
        ProjectRecordDoc,
        ProviderConnectionRecordDoc,
        ProviderConnectionListResponseDoc,
        ProviderBindingRecordDoc,
        ProviderBindingListResponseDoc,
        BundleEnvelope
    )),
    tags(
        (name = "health", description = "Service health and readiness"),
        (name = "runtime", description = "Sessions, runs, and tasks"),
        (name = "admin", description = "Tenant, workspace, and project administration"),
        (name = "providers", description = "Provider connections and bindings")
    )
)]
struct OpenApiDoc;

// ── AppBootstrap ────────────────────────────────────────────────────────────

pub struct AppBootstrap;

impl AppBootstrap {
    pub async fn router(config: BootstrapConfig) -> Result<Router, String> {
        let (router, _, _) = Self::router_with_runtime_and_tokens(config).await?;
        Ok(router)
    }

    pub async fn router_with_runtime(
        config: BootstrapConfig,
    ) -> Result<(Router, Arc<InMemoryServices>), String> {
        let (router, runtime, _) = Self::router_with_runtime_and_tokens(config).await?;
        Ok((router, runtime))
    }

    pub async fn router_with_runtime_and_tokens(
        config: BootstrapConfig,
    ) -> Result<(Router, Arc<InMemoryServices>, Arc<ServiceTokenRegistry>), String> {
        let (router, runtime, _graph, service_tokens) =
            Self::router_with_runtime_graph_and_tokens(config).await?;
        Ok((router, runtime, service_tokens))
    }

    pub async fn router_with_runtime_graph_and_tokens(
        config: BootstrapConfig,
    ) -> Result<
        (
            Router,
            Arc<InMemoryServices>,
            Arc<InMemoryGraphStore>,
            Arc<ServiceTokenRegistry>,
        ),
        String,
    > {
        let state = Arc::new(AppState::new(config).await?);
        let runtime = state.runtime.clone();
        let graph = state.graph.clone();
        let service_tokens = state.service_tokens.clone();
        let router = Self::build_router(state.clone());
        state.metrics.mark_started();
        Ok((router, runtime, graph, service_tokens))
    }

    /// Build a router around a caller-provided runtime. Integration-test
    /// entry point; production callers use [`Self::router`] et al.
    ///
    /// Lets test fixtures (see `crates/cairn-app/tests/support/fake_fabric.rs`)
    /// stand up an AppState without a live Valkey by injecting the
    /// read-only trio via `InMemoryServices::with_store_and_core` and
    /// passing it through here.
    pub async fn router_with_injected_runtime(
        config: BootstrapConfig,
        runtime: Arc<InMemoryServices>,
        fabric: Option<Arc<cairn_fabric::FabricServices>>,
    ) -> Result<(Router, Arc<AppState>), String> {
        let state = Arc::new(AppState::new_with_runtime(config, runtime, fabric).await?);
        let router = Self::build_router(state.clone());
        state.metrics.mark_started();
        // Tests that inject a runtime aren't exercising the RFC 020
        // startup graph — they skip the main.rs recovery sequence by
        // construction — so flip readiness synchronously here. Tests
        // that DO want to observe the recovery transition use
        // `LiveHarness`, which spawns the real `cairn-app` binary and
        // hits the real startup sequence.
        state.readiness.mark_ready();
        Ok((router, state))
    }

    /// Build the catalog-driven routes WITHOUT state resolution or middleware.
    ///
    /// Returns a `Router<Arc<AppState>>` so callers can `.route()` additional
    /// handlers that share the same `State<Arc<AppState>>`, then resolve state
    /// and apply middleware:
    ///
    /// ```ignore
    /// let routes = AppBootstrap::build_catalog_routes()
    ///     .route("/v1/extra", get(my_handler))
    ///     .fallback(not_found_handler)
    ///     .with_state(state.clone());
    /// let app = AppBootstrap::apply_middleware(routes, state);
    /// ```
    pub fn build_catalog_routes() -> Router<Arc<AppState>> {
        let router: Router<Arc<AppState>> = preserved_route_catalog()
            .into_iter()
            .fold(Router::new(), |router, entry| {
                let path = catalog_path_to_axum(&entry.path);
                match (entry.method, entry.path.as_str()) {
                    (HttpMethod::Get, "/health") => router.route(&path, get(health_handler)),
                    (HttpMethod::Get, "/v1/onboarding/status") => {
                        router.route(&path, get(get_onboarding_status_handler))
                    }
                    (HttpMethod::Get, "/v1/onboarding/templates") => {
                        router.route(&path, get(list_onboarding_templates_handler))
                    }
                    (HttpMethod::Post, "/v1/onboarding/template") => {
                        router.route(&path, post(materialize_onboarding_template_handler))
                    }
                    (HttpMethod::Get, "/v1/settings") => {
                        router.route(&path, get(get_settings_handler))
                    }
                    (HttpMethod::Get, "/v1/settings/tls") => {
                        router.route(&path, get(get_tls_settings_handler))
                    }
                    (HttpMethod::Put, "/v1/settings/defaults/:scope/:scope_id/:key") => {
                        router.route(&path, put(set_default_setting_handler))
                    }
                    (HttpMethod::Delete, "/v1/settings/defaults/:scope/:scope_id/:key") => {
                        router.route(&path, delete(clear_default_setting_handler))
                    }
                    (HttpMethod::Get, "/v1/settings/defaults/resolve/:key") => {
                        router.route(&path, get(resolve_default_setting_handler))
                    }
                    (HttpMethod::Get, "/v1/stream") | (HttpMethod::Get, "/v1/streams/runtime") => {
                        router.route(&path, get(runtime_stream_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/license") => {
                        router.route(&path, get(get_license_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/license/override") => {
                        router.route(&path, post(set_license_override_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/tenants") => {
                        router.route(&path, get(list_tenants_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/tenants") => {
                        router.route(&path, post(create_tenant_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/audit-log") => {
                        router.route(&path, get(list_audit_log_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/audit-log/:resource_type/:resource_id") => {
                        router.route(&path, get(list_audit_log_for_resource_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/logs") => {
                        router.route(&path, get(list_request_logs_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/tenants/:id") => {
                        router.route(&path, get(get_tenant_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/tenants/:id/overview") => {
                        router.route(&path, get(get_tenant_overview_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/tenants/:id/compact-event-log") => {
                        router.route(&path, post(compact_event_log_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/tenants/:id/snapshot") => {
                        router.route(&path, post(create_snapshot_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/tenants/:id/snapshots") => {
                        router.route(&path, get(list_snapshots_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/tenants/:id/restore") => {
                        router.route(&path, post(restore_from_snapshot_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/tenants/:tenant_id/workspaces") => {
                        router.route(&path, get(list_workspaces_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/tenants/:tenant_id/workspaces") => {
                        router.route(&path, post(create_workspace_handler))
                    }
                    (
                        HttpMethod::Delete,
                        "/v1/admin/tenants/:tenant_id/workspaces/:workspace_id",
                    ) => router.route(&path, delete(delete_workspace_handler)),
                    (
                        HttpMethod::Delete,
                        "/v1/admin/tenants/:tenant_id/sessions/:session_id",
                    ) => router.route(&path, delete(delete_session_admin_handler)),
                    (HttpMethod::Get, "/v1/admin/tenants/:tenant_id/operator-profiles") => {
                        router.route(&path, get(list_operator_profiles_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/tenants/:tenant_id/operator-profiles") => {
                        router.route(&path, post(create_operator_profile_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/operators/:id/notifications") => {
                        router.route(&path, post(set_operator_notifications_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/operators/:id/notifications") => {
                        router.route(&path, get(get_operator_notifications_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/notifications/failed") => {
                        router.route(&path, get(list_failed_notifications_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/notifications/:id/retry") => {
                        router.route(&path, post(retry_notification_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/tenants/:tenant_id/credentials") => {
                        router.route(&path, get(list_credentials_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/tenants/:tenant_id/credentials") => {
                        router.route(&path, post(store_credential_handler))
                    }
                    (HttpMethod::Delete, "/v1/admin/tenants/:tenant_id/credentials/:id") => {
                        router.route(&path, delete(revoke_credential_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/workspaces/:workspace_id/projects") => {
                        router.route(&path, get(list_projects_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/workspaces/:workspace_id/projects") => {
                        router.route(&path, post(create_project_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/workspaces/:workspace_id/members") => {
                        router.route(&path, get(list_workspace_members_handler))
                    }
                    (HttpMethod::Post, "/v1/admin/workspaces/:workspace_id/members") => {
                        router.route(&path, post(add_workspace_member_handler))
                    }
                    (
                        HttpMethod::Delete,
                        "/v1/admin/workspaces/:workspace_id/members/:member_id",
                    ) => router.route(&path, delete(remove_workspace_member_handler)),
                    (HttpMethod::Post, "/v1/admin/workspaces/:id/shares") => {
                        router.route(&path, post(create_workspace_share_handler))
                    }
                    (HttpMethod::Get, "/v1/admin/workspaces/:id/shares") => {
                        router.route(&path, get(list_workspace_shares_handler))
                    }
                    (HttpMethod::Delete, "/v1/admin/workspaces/:id/shares/:share_id") => {
                        router.route(&path, delete(revoke_workspace_share_handler))
                    }
                    (HttpMethod::Get, "/v1/prompts/assets") => {
                        router.route(&path, get(list_prompt_assets_handler))
                    }
                    (HttpMethod::Post, "/v1/prompts/assets") => {
                        router.route(&path, post(create_prompt_asset_handler))
                    }
                    (HttpMethod::Get, "/v1/prompts/assets/:id/versions") => {
                        router.route(&path, get(list_prompt_versions_handler))
                    }
                    (HttpMethod::Post, "/v1/prompts/assets/:id/versions") => {
                        router.route(&path, post(create_prompt_version_handler))
                    }
                    (HttpMethod::Get, "/v1/prompts/releases") => {
                        router.route(&path, get(list_prompt_releases_handler))
                    }
                    (HttpMethod::Post, "/v1/prompts/releases") => {
                        router.route(&path, post(create_prompt_release_handler))
                    }
                    (HttpMethod::Post, "/v1/prompts/releases/:id/transition") => {
                        router.route(&path, post(transition_prompt_release_handler))
                    }
                    (HttpMethod::Post, "/v1/prompts/releases/:id/activate") => {
                        router.route(&path, post(activate_prompt_release_handler))
                    }
                    (HttpMethod::Post, "/v1/prompts/releases/:id/rollback") => {
                        router.route(&path, post(rollback_prompt_release_handler))
                    }
                    (HttpMethod::Post, "/v1/prompts/releases/:id/rollout") => {
                        router.route(&path, post(start_prompt_rollout_handler))
                    }
                    (HttpMethod::Post, "/v1/prompts/releases/:id/request-approval") => {
                        router.route(&path, post(request_prompt_release_approval_handler))
                    }
                    (HttpMethod::Get, "/v1/approvals") => {
                        router.route(&path, get(list_approvals_handler))
                    }
                    (HttpMethod::Get, "/v1/approval-policies") => {
                        router.route(&path, get(list_approval_policies_handler))
                    }
                    (HttpMethod::Post, "/v1/approval-policies") => {
                        router.route(&path, post(create_approval_policy_handler))
                    }
                    (HttpMethod::Post, "/v1/approvals/:id/approve") => {
                        router.route(&path, post(approve_approval_handler))
                    }
                    (HttpMethod::Post, "/v1/approvals/:id/deny")
                    | (HttpMethod::Post, "/v1/approvals/:id/reject") => {
                        router.route(&path, post(reject_approval_handler))
                    }
                    (HttpMethod::Post, "/v1/approvals/:id/delegate") => {
                        router.route(&path, post(delegate_approval_handler))
                    }
                    // ── Plan review (RFC 018) ────────────────────────────────
                    (HttpMethod::Post, "/v1/runs/:id/approve") => {
                        router.route(&path, post(approve_plan_handler))
                    }
                    (HttpMethod::Post, "/v1/runs/:id/reject") => {
                        router.route(&path, post(reject_plan_handler))
                    }
                    (HttpMethod::Post, "/v1/runs/:id/revise") => {
                        router.route(&path, post(revise_plan_handler))
                    }
                    // ── SQ/EQ + A2A (RFC 021) ────────────────────────────────
                    (HttpMethod::Post, "/v1/sqeq/initialize") => {
                        router.route(&path, post(sqeq_initialize_handler))
                    }
                    (HttpMethod::Post, "/v1/sqeq/submit") => {
                        router.route(&path, post(sqeq_submit_handler))
                    }
                    (HttpMethod::Get, "/v1/sqeq/events") => {
                        router.route(&path, get(sqeq_events_handler))
                    }
                    (HttpMethod::Get, "/.well-known/agent.json") => {
                        router.route(&path, get(a2a_agent_card_handler))
                    }
                    (HttpMethod::Post, "/v1/a2a/tasks") => {
                        router.route(&path, post(a2a_submit_task_handler))
                    }
                    (HttpMethod::Get, "/v1/a2a/tasks/:id") => {
                        router.route(&path, get(a2a_get_task_handler))
                    }
                    // ── Decisions (RFC 019) — handled via nest below ─────────
                    (HttpMethod::Get, "/v1/decisions")
                    | (HttpMethod::Get, "/v1/decisions/cache")
                    | (HttpMethod::Get, "/v1/decisions/:id")
                    | (HttpMethod::Post, "/v1/decisions/evaluate")
                    | (HttpMethod::Post, "/v1/decisions/:id/invalidate")
                    | (HttpMethod::Post, "/v1/decisions/invalidate")
                    | (HttpMethod::Post, "/v1/decisions/invalidate-by-rule") => router,
                    (HttpMethod::Get, "/v1/runs") => router.route(&path, get(list_runs_handler)),
                    (HttpMethod::Get, "/v1/runs/stalled") => {
                        router.route(&path, get(list_stalled_runs_handler))
                    }
                    (HttpMethod::Get, "/v1/runs/escalated") => {
                        router.route(&path, get(list_escalated_runs_handler))
                    }
                    (HttpMethod::Post, "/v1/runs/:id/cost-alert") => {
                        router.route(&path, post(set_run_cost_alert_handler))
                    }
                    (HttpMethod::Get, "/v1/runs/cost-alerts") => {
                        router.route(&path, get(list_run_cost_alerts_handler))
                    }
                    (HttpMethod::Post, "/v1/runs/:id/sla") => {
                        router.route(&path, post(set_run_sla_handler))
                    }
                    (HttpMethod::Get, "/v1/runs/:id/sla") => {
                        router.route(&path, get(get_run_sla_handler))
                    }
                    (HttpMethod::Get, "/v1/runs/sla-breached") => {
                        router.route(&path, get(list_sla_breached_handler))
                    }
                    (HttpMethod::Post, "/v1/runs/:id/diagnose") => {
                        router.route(&path, post(diagnose_run_handler))
                    }
                    (HttpMethod::Get, "/v1/runs/:id/interventions") => {
                        router.route(&path, get(list_run_interventions_handler))
                    }
                    (HttpMethod::Get, "/v1/costs") => {
                        router.route(&path, get(list_tenant_costs_handler))
                    }
                    (HttpMethod::Get, "/v1/runs/resume-due") => {
                        router.route(&path, get(list_due_run_resumes_handler))
                    }
                    (HttpMethod::Post, "/v1/runs/process-scheduled-resumes") => {
                        router.route(&path, post(process_scheduled_run_resumes_handler))
                    }
                    (HttpMethod::Post, "/v1/runs/:id/intervene") => {
                        router.route(&path, post(intervene_run_handler))
                    }
                    (HttpMethod::Get, "/v1/tool-invocations") => {
                        router.route(&path, get(list_tool_invocations_handler))
                    }
                    (HttpMethod::Get, "/v1/tool-invocations/:id") => {
                        router.route(&path, get(get_tool_invocation_handler))
                    }
                    (HttpMethod::Get, "/v1/tool-invocations/:id/progress") => {
                        router.route(&path, get(get_tool_invocation_progress_handler))
                    }
                    (HttpMethod::Post, "/v1/tool-invocations") => {
                        router.route(&path, post(create_tool_invocation_handler))
                    }
                    (HttpMethod::Post, "/v1/tool-invocations/:id/cancel") => {
                        router.route(&path, post(cancel_tool_invocation_handler))
                    }
                    (HttpMethod::Get, "/v1/checkpoints") => {
                        router.route(&path, get(list_checkpoints_handler))
                    }
                    (HttpMethod::Get, "/v1/checkpoints/:id") => {
                        router.route(&path, get(get_checkpoint_handler))
                    }
                    (HttpMethod::Post, "/v1/runs/:id/checkpoint") => {
                        router.route(&path, post(save_checkpoint_handler))
                    }
                    (HttpMethod::Get, "/v1/plugins") => {
                        router.route(&path, get(list_plugins_handler))
                    }
                    (HttpMethod::Post, "/v1/plugins") => {
                        router.route(&path, post(create_plugin_handler))
                    }
                    (HttpMethod::Get, "/v1/plugins/:id") => {
                        router.route(&path, get(get_plugin_handler))
                    }
                    (HttpMethod::Delete, "/v1/plugins/:id") => {
                        router.route(&path, delete(delete_plugin_handler))
                    }
                    (HttpMethod::Get, "/v1/plugins/:id/health") => {
                        router.route(&path, get(plugin_health_handler))
                    }
                    (HttpMethod::Get, "/v1/plugins/:id/metrics") => {
                        router.route(&path, get(plugin_metrics_handler))
                    }
                    (HttpMethod::Get, "/v1/plugins/:id/logs") => {
                        router.route(&path, get(plugin_logs_handler))
                    }
                    (HttpMethod::Get, "/v1/plugins/:id/pending-signals") => {
                        router.route(&path, get(plugin_pending_signals_handler))
                    }
                    (HttpMethod::Post, "/v1/plugins/:id/eval-score") => {
                        router.route(&path, post(plugin_eval_score_handler))
                    }
                    (HttpMethod::Get, "/v1/plugins/:id/capabilities") => {
                        router.route(&path, get(plugin_capabilities_handler))
                    }
                    (HttpMethod::Get, "/v1/plugins/:id/tools") => {
                        router.route(&path, get(plugin_tools_handler))
                    }
                    (HttpMethod::Get, "/v1/plugins/tools/search") => {
                        router.route(&path, get(plugin_tools_search_handler))
                    }
                    (HttpMethod::Get, "/v1/runs/:id") => router.route(&path, get(get_run_handler)),
                    (HttpMethod::Get, "/v1/runs/:id/audit") => {
                        router.route(&path, get(get_run_audit_trail_handler))
                    }
                    (HttpMethod::Get, "/v1/mailbox") => router,
                    (HttpMethod::Get, "/v1/feed") => router.route(&path, get(list_feed_handler)),
                    (HttpMethod::Post, "/v1/feed/:id/read") => {
                        router.route(&path, post(mark_feed_item_read_handler))
                    }
                    (HttpMethod::Post, "/v1/feed/read-all") => {
                        router.route(&path, post(mark_all_feed_items_read_handler))
                    }
                    (HttpMethod::Get, "/v1/tasks") => router.route(&path, get(list_tasks_handler)),
                    (HttpMethod::Post, "/v1/tasks/:id/release-lease") => {
                        router.route(&path, post(release_task_lease_handler))
                    }
                    (HttpMethod::Post, "/v1/tasks/:id/priority") => {
                        router.route(&path, post(set_task_priority_handler))
                    }
                    (HttpMethod::Get, "/v1/tasks/expired") => {
                        router.route(&path, get(list_expired_tasks_handler))
                    }
                    (HttpMethod::Post, "/v1/tasks/expire-leases") => {
                        router.route(&path, post(expire_task_leases_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/dashboard") => {
                        router.route(&path, get(get_eval_dashboard_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/runs") => {
                        router.route(&path, get(list_eval_runs_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/datasets") => {
                        router.route(&path, get(list_eval_datasets_handler))
                    }
                    (HttpMethod::Post, "/v1/evals/datasets") => {
                        router.route(&path, post(create_eval_dataset_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/datasets/:id") => {
                        router.route(&path, get(get_eval_dataset_handler))
                    }
                    (HttpMethod::Post, "/v1/evals/datasets/:id/entries") => {
                        router.route(&path, post(add_eval_dataset_entry_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/baselines") => {
                        router.route(&path, get(list_eval_baselines_handler))
                    }
                    (HttpMethod::Post, "/v1/evals/baselines") => {
                        router.route(&path, post(create_eval_baseline_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/baselines/:id") => {
                        router.route(&path, get(get_eval_baseline_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/rubrics") => {
                        router.route(&path, get(list_eval_rubrics_handler))
                    }
                    (HttpMethod::Post, "/v1/evals/rubrics") => {
                        router.route(&path, post(create_eval_rubric_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/rubrics/:id") => {
                        router.route(&path, get(get_eval_rubric_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/runs/:id") => {
                        router.route(&path, get(get_eval_run_handler))
                    }
                    (HttpMethod::Post, "/v1/evals/runs") => {
                        router.route(&path, post(create_eval_run_handler))
                    }
                    (HttpMethod::Post, "/v1/evals/runs/:id/score-rubric") => {
                        router.route(&path, post(score_eval_rubric_handler))
                    }
                    (HttpMethod::Post, "/v1/evals/runs/:id/compare-baseline") => {
                        router.route(&path, post(compare_eval_baseline_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/scorecard/:asset_id") => {
                        router.route(&path, get(get_scorecard_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/assets/:asset_id/trend") => {
                        router.route(&path, get(get_eval_asset_trend_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/assets/:asset_id/winner") => {
                        router.route(&path, get(get_eval_asset_winner_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/assets/:asset_id/export") => {
                        router.route(&path, get(get_eval_asset_export_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/assets/:asset_id/report") => {
                        router.route(&path, get(get_eval_asset_report_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/matrices/prompt-comparison") => {
                        router.route(&path, get(get_prompt_comparison_matrix_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/matrices/permissions") => {
                        router.route(&path, get(get_permission_matrix_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/matrices/skill-health") => {
                        router.route(&path, get(get_skill_health_matrix_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/matrices/provider-routing") => {
                        router.route(&path, get(get_provider_routing_matrix_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/matrices/memory-quality") => {
                        router.route(&path, get(get_memory_quality_matrix_handler))
                    }
                    (HttpMethod::Get, "/v1/evals/matrices/guardrail") => {
                        router.route(&path, get(get_guardrail_matrix_handler))
                    }
                    (HttpMethod::Get, "/v1/sources") => {
                        router.route(&path, get(list_sources_handler))
                    }
                    (HttpMethod::Post, "/v1/sources") => {
                        router.route(&path, post(create_source_handler))
                    }
                    (HttpMethod::Get, "/v1/sources/:id") => {
                        router.route(&path, get(get_source_handler))
                    }
                    (HttpMethod::Put, "/v1/sources/:id") => {
                        router.route(&path, put(update_source_handler))
                    }
                    (HttpMethod::Delete, "/v1/sources/:id") => {
                        router.route(&path, delete(delete_source_handler))
                    }
                    (HttpMethod::Get, "/v1/sources/:id/chunks") => {
                        router.route(&path, get(list_source_chunks_handler))
                    }
                    (HttpMethod::Get, "/v1/sources/:id/refresh-schedule") => {
                        router.route(&path, get(get_source_refresh_schedule_handler))
                    }
                    (HttpMethod::Post, "/v1/sources/:id/refresh-schedule") => {
                        router.route(&path, post(create_source_refresh_schedule_handler))
                    }
                    (HttpMethod::Post, "/v1/sources/process-refresh") => {
                        router.route(&path, post(process_source_refresh_handler))
                    }
                    (HttpMethod::Get, "/v1/sources/:id/quality") => {
                        router.route(&path, get(source_quality_handler))
                    }
                    (HttpMethod::Get, "/v1/ingest/jobs") => {
                        router.route(&path, get(list_ingest_jobs_handler))
                    }
                    (HttpMethod::Post, "/v1/ingest/jobs") => {
                        router.route(&path, post(create_ingest_job_handler))
                    }
                    (HttpMethod::Get, "/v1/ingest/jobs/:id") => {
                        router.route(&path, get(get_ingest_job_handler))
                    }
                    (HttpMethod::Post, "/v1/ingest/jobs/:id/complete") => {
                        router.route(&path, post(complete_ingest_job_handler))
                    }
                    (HttpMethod::Post, "/v1/ingest/jobs/:id/fail") => {
                        router.route(&path, post(fail_ingest_job_handler))
                    }
                    (HttpMethod::Get, "/v1/channels") => {
                        router.route(&path, get(list_channels_handler))
                    }
                    (HttpMethod::Post, "/v1/channels") => {
                        router.route(&path, post(create_channel_handler))
                    }
                    (HttpMethod::Post, "/v1/channels/:id/send") => {
                        router.route(&path, post(send_channel_message_handler))
                    }
                    (HttpMethod::Post, "/v1/channels/:id/consume") => {
                        router.route(&path, post(consume_channel_message_handler))
                    }
                    (HttpMethod::Get, "/v1/channels/:id/messages") => {
                        router.route(&path, get(list_channel_messages_handler))
                    }
                    (HttpMethod::Get, "/v1/memories") => {
                        router.route(&path, get(list_memories_preserved_handler))
                    }
                    (HttpMethod::Get, "/v1/memories/search") => {
                        router.route(&path, get(search_memories_preserved_handler))
                    }
                    (HttpMethod::Post, "/v1/memories") => {
                        router.route(&path, post(create_memory_preserved_handler))
                    }
                    (HttpMethod::Get, "/v1/graph/trace") => {
                        router.route(&path, get(graph_trace_preserved_handler))
                    }
                    (HttpMethod::Get, "/v1/skills") => {
                        router.route(&path, get(list_skills_handler))
                    }
                    // `/v1/skills/:id` is registered in the dynamic-param
                    // `.route()` chain below — matchit 0.7 rejects the
                    // `{id}` literal produced by `catalog_path_to_axum`.
                    // Skip the fold without registering a no-op 501
                    // shadow; returning `router` unchanged leaves the
                    // dynamic-chain registration as the authoritative
                    // binding. The catalog entry still contributes to
                    // OpenAPI discovery because it's iterated directly.
                    (HttpMethod::Get, "/v1/skills/:id") => router,
                    (HttpMethod::Get, "/v1/memory/search") => {
                        router.route(&path, get(memory_search_handler))
                    }
                    (HttpMethod::Post, "/v1/memory/ingest") => {
                        router.route(&path, post(memory_ingest_handler))
                    }
                    (HttpMethod::Post, "/v1/memory/deep-search") => {
                        router.route(&path, post(memory_deep_search_handler))
                    }
                    (HttpMethod::Get, "/v1/memory/diagnostics") => {
                        router.route(&path, get(memory_diagnostics_handler))
                    }
                    (HttpMethod::Get, "/v1/memory/provenance/:document_id") => {
                        router.route(&path, get(memory_provenance_handler))
                    }
                    (HttpMethod::Get, "/v1/dashboard") => {
                        router.route(&path, get(dashboard_handler))
                    }
                    (HttpMethod::Get, "/v1/trace/:trace_id") => {
                        router.route(&path, get(get_trace_handler))
                    }
                    (HttpMethod::Get, "/v1/graph/execution-trace/:run_id") => {
                        router.route(&path, get(execution_trace_handler))
                    }
                    (HttpMethod::Get, "/v1/graph/retrieval-provenance/:run_id") => {
                        router.route(&path, get(retrieval_provenance_handler))
                    }
                    (HttpMethod::Get, "/v1/graph/prompt-provenance/:release_id") => {
                        router.route(&path, get(prompt_provenance_handler))
                    }
                    (HttpMethod::Get, "/v1/graph/dependency-path/:run_id") => {
                        router.route(&path, get(dependency_path_handler))
                    }
                    (HttpMethod::Get, "/v1/graph/provenance/:node_id") => {
                        router.route(&path, get(graph_provenance_handler))
                    }
                    (HttpMethod::Get, "/v1/providers/health") => {
                        router.route(&path, get(list_provider_health_handler))
                    }
                    (HttpMethod::Get, "/v1/providers/budget") => {
                        router.route(&path, get(list_provider_budgets_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/budget") => {
                        router.route(&path, post(set_provider_budget_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/:id/health-check") => {
                        router.route(&path, post(manual_provider_health_check_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/:id/recover") => {
                        router.route(&path, post(recover_provider_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/pools") => {
                        router.route(&path, post(create_provider_pool_handler))
                    }
                    (HttpMethod::Get, "/v1/providers/pools") => {
                        router.route(&path, get(list_provider_pools_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/pools/:id/connections") => {
                        router.route(&path, post(add_pool_connection_handler))
                    }
                    (HttpMethod::Delete, "/v1/providers/pools/:id/connections/:conn_id") => {
                        router.route(&path, delete(remove_pool_connection_handler))
                    }
                    (HttpMethod::Get, "/v1/providers/connections") => {
                        router.route(&path, get(list_provider_connections_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/connections") => {
                        router.route(&path, post(create_provider_connection_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/connections/:id/models") => {
                        router.route(&path, post(register_provider_model_handler))
                    }
                    (HttpMethod::Get, "/v1/providers/connections/:id/models") => {
                        router.route(&path, get(list_provider_models_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/connections/:id/health-schedule") => {
                        router.route(&path, post(set_provider_health_schedule_handler))
                    }
                    (HttpMethod::Get, "/v1/providers/connections/:id/health-schedule") => {
                        router.route(&path, get(get_provider_health_schedule_handler))
                    }
                    (HttpMethod::Put, "/v1/providers/connections/:id/retry-policy") => {
                        router.route(&path, put(set_provider_retry_policy_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/run-health-checks") => {
                        router.route(&path, post(run_provider_health_checks_handler))
                    }
                    (HttpMethod::Get, "/v1/providers/bindings") => {
                        router.route(&path, get(list_provider_bindings_handler))
                    }
                    (HttpMethod::Get, "/v1/providers/bindings/:id/cost-stats") => {
                        router.route(&path, get(get_binding_cost_stats_handler))
                    }
                    (HttpMethod::Get, "/v1/providers/bindings/cost-ranking") => {
                        router.route(&path, get(list_binding_cost_ranking_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/bindings") => {
                        router.route(&path, post(create_provider_binding_handler))
                    }
                    (HttpMethod::Get, "/v1/providers/policies") => {
                        router.route(&path, get(list_route_policies_handler))
                    }
                    (HttpMethod::Post, "/v1/providers/policies") => {
                        router.route(&path, post(create_route_policy_handler))
                    }
                    (HttpMethod::Get, "/v1/status") => {
                        router.route(&path, get(system_status_handler))
                    }
                    (HttpMethod::Get, "/v1/sessions/:id/llm-traces") => {
                        router.route(&path, get(get_session_llm_traces_handler))
                    }
                    (HttpMethod::Get, "/v1/fleet") => router.route(&path, get(fleet_handler)),
                    (HttpMethod::Get, "/v1/overview") => {
                        router.route(&path, get(system_status_handler))
                    }
                    (HttpMethod::Get, "/v1/metrics") => router.route(&path, get(metrics_handler)),
                    (HttpMethod::Get, _) => router.route(&path, get(not_implemented_handler)),
                    (HttpMethod::Post, _) => router.route(&path, post(not_implemented_handler)),
                    (HttpMethod::Put, _) => router.route(&path, put(not_implemented_handler)),
                    (HttpMethod::Delete, _) => router.route(&path, delete(not_implemented_handler)),
                    (HttpMethod::Patch, _) => router.route(&path, patch(not_implemented_handler)),
                }
            })
            .route("/ready", get(ready_handler))
            .route("/health/ready", get(health_ready_handler))
            .route("/metrics", get(metrics_handler))
            .route("/version", get(version_handler))
            .route("/v1/dashboard/activity", get(dashboard_activity_handler))
            .route("/v1/agent-templates", get(list_agent_templates_handler))
            .route(
                "/v1/agent-templates/:id/instantiate",
                post(instantiate_agent_template_handler),
            )
            .route(
                "/v1/sessions",
                get(list_sessions_handler).post(create_session_handler),
            )
            .route("/v1/sessions/:id", get(get_session_handler))
            .route("/v1/sessions/:id/cost", get(get_session_cost_handler))
            .route(
                "/v1/sessions/:id/activity",
                get(get_session_activity_handler),
            )
            .route("/v1/sessions/:id/events", get(list_session_events_handler))
            .route(
                "/v1/sessions/:id/active-runs",
                get(get_session_active_runs_handler),
            )
            .route("/v1/runs", post(create_run_handler))
            .route("/v1/runs/:id/audit", get(get_run_audit_trail_handler))
            .route("/v1/runs/:id/cost", get(get_run_cost_handler))
            .route("/v1/runs/:id/recover", post(recover_run_handler))
            .route("/v1/runs/:id/events", get(list_run_events_handler))
            .route("/v1/runs/:id/replay", get(replay_run_handler))
            .route(
                "/v1/runs/:id/replay-to-checkpoint",
                post(replay_run_to_checkpoint_handler),
            )
            .route("/v1/runs/:id/claim", post(claim_run_handler))
            .route("/v1/runs/:id/cancel", post(cancel_run_handler))
            .route("/v1/runs/:id/pause", post(pause_run_handler))
            .route("/v1/runs/:id/resume", post(resume_run_handler))
            // Plan review (RFC 018)
            .route("/v1/runs/:id/approve", post(approve_plan_handler))
            .route("/v1/runs/:id/reject", post(reject_plan_handler))
            .route("/v1/runs/:id/revise", post(revise_plan_handler))
            .route(
                "/v1/runs/:id/checkpoint-strategy",
                get(get_checkpoint_strategy_handler).post(set_checkpoint_strategy_handler),
            )
            .route("/v1/runs/:id/spawn", post(spawn_subagent_run_handler))
            .route("/v1/runs/:id/children", get(list_child_runs_handler))
            .route("/v1/runs/:id/orchestrate", post(orchestrate_run_handler))
            .route(
                "/v1/plugins/:id/capabilities",
                get(plugin_capabilities_handler),
            )
            .route("/v1/plugins/:id/tools", get(plugin_tools_handler))
            .route("/v1/plugins/tools/search", get(plugin_tools_search_handler))
            .route("/v1/evals/dashboard", get(get_eval_dashboard_handler))
            .route(
                "/v1/evals/matrices/provider-routing",
                get(get_provider_routing_matrix_handler),
            )
            .route("/v1/evals/runs/:id/start", post(start_eval_run_handler))
            .route(
                "/v1/evals/runs/:id/complete",
                post(complete_eval_run_handler),
            )
            .route("/v1/evals/runs/:id/score", post(score_eval_run_handler))
            .route("/v1/evals/compare", get(compare_eval_runs_handler))
            .route("/v1/memory/feedback", post(memory_feedback_handler))
            .route("/v1/memory/documents/:id", get(get_memory_document_handler))
            .route(
                "/v1/memory/documents/:id/versions",
                get(list_memory_document_versions_handler),
            )
            .route(
                "/v1/memory/related/:document_id",
                get(memory_related_documents_handler),
            )
            .route(
                "/v1/prompts/releases/compare",
                post(compare_prompt_releases_handler),
            )
            .route(
                "/v1/prompts/releases/:id/history",
                get(prompt_release_history_handler),
            )
            .route(
                "/v1/prompts/assets/:id/versions/:version_id/diff",
                get(diff_prompt_versions_handler),
            )
            .route(
                "/v1/prompts/assets/:id/versions/:version_id/render",
                post(render_prompt_version_handler),
            )
            .route(
                "/v1/prompts/assets/:id/versions/:version_id/template-vars",
                get(list_prompt_template_vars_handler),
            )
            .route("/v1/approvals", post(request_approval_handler))
            .route("/v1/policies", post(create_guardrail_policy_handler))
            .route(
                "/v1/policies/evaluate",
                post(evaluate_guardrail_policy_handler),
            )
            .route(
                "/v1/mailbox",
                get(list_mailbox_handler).post(append_mailbox_handler),
            )
            .route(
                "/v1/signals",
                post(ingest_signal_handler).get(list_signals_handler),
            )
            .route(
                "/v1/signals/subscriptions",
                post(create_signal_subscription_handler).get(list_signal_subscriptions_handler),
            )
            .route("/v1/tasks", post(create_task_handler))
            .route("/v1/tasks/:id", get(get_task_handler))
            .route(
                "/v1/tasks/:id/dependencies",
                get(list_task_dependencies_handler).post(add_task_dependency_handler),
            )
            .route("/v1/tasks/:id/claim", post(claim_task_handler))
            .route("/v1/tasks/:id/heartbeat", post(heartbeat_task_handler))
            .route("/v1/tasks/:id/complete", post(complete_task_handler))
            // NOTE: POST /v1/tasks/expire-leases is registered via the preserved_route_catalog fold
            .route(
                "/v1/tool-invocations/:id/complete",
                post(complete_tool_invocation_handler),
            )
            .route("/v1/bundles/validate", post(validate_bundle_handler))
            .route("/v1/bundles/plan", post(plan_bundle_handler))
            .route("/v1/bundles/apply", post(apply_bundle_handler))
            .route("/v1/bundles/export", get(export_bundle_handler))
            .route(
                "/v1/bundles/export-filtered",
                post(export_filtered_bundle_handler),
            )
            .route(
                "/v1/bundles/export/prompts",
                get(export_prompt_bundle_handler),
            )
            .route("/v1/mailbox/:id", delete(mark_mailbox_delivered_handler))
            .route(
                "/v1/signals/subscriptions/:id",
                delete(delete_signal_subscription_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/credentials/rotate-key",
                post(rotate_credential_key_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/quota",
                get(get_tenant_quota_handler).post(set_tenant_quota_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/retention-policy",
                get(get_retention_policy_handler).post(set_retention_policy_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/apply-retention",
                post(apply_retention_handler),
            )
            .route("/v1/workers/register", post(register_worker_handler))
            .route("/v1/workers", get(list_workers_handler))
            .route("/v1/workers/:id", get(get_worker_handler))
            .route("/v1/workers/:id/claim", post(worker_claim_task_handler))
            .route("/v1/workers/:id/report", post(worker_report_handler))
            .route("/v1/workers/:id/heartbeat", post(worker_heartbeat_handler))
            .route("/v1/workers/:id/suspend", post(suspend_worker_handler))
            .route(
                "/v1/workers/:id/reactivate",
                post(reactivate_worker_handler),
            )
            .route("/openapi.json", get(openapi_json_handler))
            .route("/docs", get(swagger_docs_handler))
            // ── Dynamic-path routes ──────────────────────────────────────────────────
            // catalog_path_to_axum(:id → {id}) produces a static literal in matchit 0.7,
            // so ALL dynamic-param routes must be registered here with :param syntax.
            // ── Admin GET ────────────────────────────────────────────────────────────
            .route(
                "/v1/admin/audit-log/:resource_type/:resource_id",
                get(list_audit_log_for_resource_handler),
            )
            .route("/v1/admin/tenants/:id", get(get_tenant_handler))
            .route(
                "/v1/admin/tenants/:id/overview",
                get(get_tenant_overview_handler),
            )
            .route(
                "/v1/admin/tenants/:id/snapshots",
                get(list_snapshots_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/credentials",
                get(list_credentials_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/operator-profiles",
                get(list_operator_profiles_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/workspaces",
                get(list_workspaces_handler),
            )
            .route(
                "/v1/admin/workspaces/:workspace_id/members",
                get(list_workspace_members_handler),
            )
            .route(
                "/v1/admin/workspaces/:workspace_id/projects",
                get(list_projects_handler),
            )
            .route(
                "/v1/admin/workspaces/:id/shares",
                get(list_workspace_shares_handler),
            )
            .route(
                "/v1/admin/operators/:id/notifications",
                get(get_operator_notifications_handler),
            )
            // ── Admin POST/DELETE ─────────────────────────────────────────────────────
            .route(
                "/v1/admin/tenants/:id/compact-event-log",
                post(compact_event_log_handler),
            )
            .route(
                "/v1/admin/tenants/:id/snapshot",
                post(create_snapshot_handler),
            )
            .route(
                "/v1/admin/tenants/:id/restore",
                post(restore_from_snapshot_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/workspaces",
                post(create_workspace_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/workspaces/:workspace_id",
                delete(delete_workspace_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/sessions/:session_id",
                delete(delete_session_admin_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/operator-profiles",
                post(create_operator_profile_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/credentials",
                post(store_credential_handler),
            )
            .route(
                "/v1/admin/tenants/:tenant_id/credentials/:id",
                delete(revoke_credential_handler),
            )
            .route(
                "/v1/admin/workspaces/:workspace_id/projects",
                post(create_project_handler),
            )
            .route(
                "/v1/admin/workspaces/:workspace_id/members",
                post(add_workspace_member_handler),
            )
            .route(
                "/v1/admin/workspaces/:workspace_id/members/:id",
                delete(remove_workspace_member_handler),
            )
            .route(
                "/v1/admin/workspaces/:id/shares",
                post(create_workspace_share_handler),
            )
            .route(
                "/v1/admin/workspaces/:id/shares/:share_id",
                delete(revoke_workspace_share_handler),
            )
            .route(
                "/v1/admin/operators/:id/notifications",
                post(set_operator_notifications_handler),
            )
            .route(
                "/v1/admin/notifications/:id/retry",
                post(retry_notification_handler),
            )
            // ── Model pricing admin (CRUD) ────────────────────────────────────────────
            .route("/v1/admin/models", get(list_models_handler))
            .route(
                "/v1/admin/models/import-litellm",
                post(import_litellm_handler),
            )
            .route(
                "/v1/admin/models/:id",
                get(get_model_handler)
                    .put(set_model_handler)
                    .delete(delete_model_handler),
            )
            // ── Settings ──────────────────────────────────────────────────────────────
            .route("/v1/settings/defaults/all", get(list_all_defaults_handler))
            .route(
                "/v1/settings/defaults/resolve/:key",
                get(resolve_default_setting_handler),
            )
            .route(
                "/v1/settings/defaults/:scope/:scope_id/:key",
                put(set_default_setting_handler).delete(clear_default_setting_handler),
            )
            // ── Approvals ─────────────────────────────────────────────────────────────
            .route("/v1/approvals/:id/approve", post(approve_approval_handler))
            .route("/v1/approvals/:id/deny", post(deny_approval_handler))
            .route(
                "/v1/approvals/:id/delegate",
                post(delegate_approval_handler),
            )
            .route("/v1/approvals/:id/reject", post(reject_approval_handler))
            // ── Decisions (RFC 019) ───────────────────────────────────────────────────
            // All decision routes use nest() to avoid static/dynamic path conflicts.
            .nest("/v1/decisions", {
                axum::Router::new()
                    .route("/", get(list_decisions_handler))
                    .route("/cache", get(list_decision_cache_handler))
                    .route("/evaluate", post(evaluate_decision_handler))
                    .route("/invalidate", post(bulk_invalidate_decisions_handler))
                    .route("/invalidate-by-rule", post(invalidate_by_rule_handler))
                    .route("/:id", get(get_decision_handler))
                    .route("/:id/invalidate", post(invalidate_decision_handler))
            })
            // SQ/EQ + A2A routes (RFC 021) are registered via the catalog fold above.
            .route("/v1/a2a/tasks/:id", get(a2a_get_task_handler))
            // ── Prompts ───────────────────────────────────────────────────────────────
            .route(
                "/v1/prompts/assets/:id/versions",
                get(list_prompt_versions_handler),
            )
            .route(
                "/v1/prompts/assets/:id/versions",
                post(create_prompt_version_handler),
            )
            .route(
                "/v1/prompts/releases/:id/transition",
                post(transition_prompt_release_handler),
            )
            .route(
                "/v1/prompts/releases/:id/activate",
                post(activate_prompt_release_handler),
            )
            .route(
                "/v1/prompts/releases/:id/rollback",
                post(rollback_prompt_release_handler),
            )
            .route(
                "/v1/prompts/releases/:id/rollout",
                post(start_prompt_rollout_handler),
            )
            .route(
                "/v1/prompts/releases/:id/request-approval",
                post(request_prompt_release_approval_handler),
            )
            // ── Feed ──────────────────────────────────────────────────────────────────
            .route("/v1/feed/:id/read", post(mark_feed_item_read_handler))
            .route(
                "/v1/telemetry/usage",
                get(telemetry_routes::get_usage_telemetry_handler),
            )
            // ── Skills ────────────────────────────────────────────────────────────────
            .route("/v1/skills/:id", get(get_skill_handler))
            // ── Runs ──────────────────────────────────────────────────────────────────
            .route("/v1/runs/:id", get(get_run_handler))
            .route("/v1/runs/:id/cost-alert", post(set_run_cost_alert_handler))
            .route(
                "/v1/runs/:id/sla",
                get(get_run_sla_handler).post(set_run_sla_handler),
            )
            .route(
                "/v1/runs/:id/interventions",
                get(list_run_interventions_handler),
            )
            .route("/v1/runs/:id/diagnose", post(diagnose_run_handler))
            .route("/v1/runs/:id/intervene", post(intervene_run_handler))
            .route("/v1/runs/:id/checkpoint", post(save_checkpoint_handler))
            // ── Tasks ─────────────────────────────────────────────────────────────────
            .route("/v1/tasks/:id/cancel", post(cancel_task_handler))
            .route(
                "/v1/tasks/:id/release-lease",
                post(release_task_lease_handler),
            )
            .route("/v1/tasks/:id/priority", post(set_task_priority_handler))
            // ── Tool invocations ──────────────────────────────────────────────────────
            .route("/v1/tool-invocations/:id", get(get_tool_invocation_handler))
            .route(
                "/v1/tool-invocations/:id/progress",
                get(get_tool_invocation_progress_handler),
            )
            .route(
                "/v1/tool-invocations/:id/cancel",
                post(cancel_tool_invocation_handler),
            )
            // ── Checkpoints ───────────────────────────────────────────────────────────
            .route("/v1/checkpoints/:id", get(get_checkpoint_handler))
            .route(
                "/v1/checkpoints/:id/restore",
                post(restore_checkpoint_handler),
            )
            // ── Plugins ───────────────────────────────────────────────────────────────
            .route(
                "/v1/plugins/:id",
                get(get_plugin_handler).delete(unregister_plugin_handler),
            )
            .route("/v1/plugins/:id/health", get(plugin_health_handler))
            .route("/v1/plugins/:id/metrics", get(plugin_metrics_handler))
            .route("/v1/plugins/:id/logs", get(plugin_logs_handler))
            .route(
                "/v1/plugins/:id/pending-signals",
                get(plugin_pending_signals_handler),
            )
            .route(
                "/v1/plugins/:id/eval-score",
                post(plugin_eval_score_handler),
            )
            // ── Evals ─────────────────────────────────────────────────────────────────
            .route("/v1/evals/datasets/:id", get(get_eval_dataset_handler))
            .route(
                "/v1/evals/datasets/:id/entries",
                post(add_eval_dataset_entry_handler),
            )
            .route("/v1/evals/baselines/:id", get(get_eval_baseline_handler))
            .route("/v1/evals/rubrics/:id", get(get_eval_rubric_handler))
            .route("/v1/evals/runs/:id", get(get_eval_run_handler))
            .route(
                "/v1/evals/runs/:id/score-rubric",
                post(score_eval_run_with_rubric_handler),
            )
            .route(
                "/v1/evals/runs/:id/compare-baseline",
                post(compare_eval_run_baseline_handler),
            )
            .route("/v1/evals/scorecard/:asset_id", get(get_scorecard_handler))
            .route(
                "/v1/evals/assets/:asset_id/report",
                get(get_eval_asset_report_handler),
            )
            .route(
                "/v1/evals/assets/:asset_id/trend",
                get(get_eval_asset_trend_handler),
            )
            .route(
                "/v1/evals/assets/:asset_id/winner",
                get(get_eval_asset_winner_handler),
            )
            .route(
                "/v1/evals/assets/:asset_id/export",
                get(get_eval_asset_export_handler),
            )
            // ── Sources / Ingest ──────────────────────────────────────────────────────
            .route(
                "/v1/sources/:id",
                get(get_source_handler)
                    .put(update_source_handler)
                    .delete(delete_source_handler),
            )
            .route("/v1/sources/:id/chunks", get(list_source_chunks_handler))
            .route("/v1/sources/:id/quality", get(source_quality_handler))
            .route(
                "/v1/sources/:id/refresh-schedule",
                get(get_source_refresh_schedule_handler)
                    .post(create_source_refresh_schedule_handler),
            )
            .route("/v1/ingest/jobs/:id", get(get_ingest_job_handler))
            .route(
                "/v1/ingest/jobs/:id/complete",
                post(complete_ingest_job_handler),
            )
            .route("/v1/ingest/jobs/:id/fail", post(fail_ingest_job_handler))
            // ── Channels ──────────────────────────────────────────────────────────────
            .route(
                "/v1/channels/:id/messages",
                get(list_channel_messages_handler),
            )
            .route("/v1/channels/:id/send", post(send_channel_message_handler))
            .route(
                "/v1/channels/:id/consume",
                post(consume_channel_message_handler),
            )
            // ── Sessions ──────────────────────────────────────────────────────────────
            .route(
                "/v1/sessions/:id/llm-traces",
                get(get_session_llm_traces_handler),
            )
            // ── Graph ─────────────────────────────────────────────────────────────────
            .route(
                "/v1/graph/execution-trace/:run_id",
                get(execution_trace_handler),
            )
            .route(
                "/v1/graph/dependency-path/:run_id",
                get(dependency_path_handler),
            )
            .route(
                "/v1/graph/prompt-provenance/:release_id",
                get(prompt_provenance_handler),
            )
            .route(
                "/v1/graph/retrieval-provenance/:run_id",
                get(retrieval_provenance_handler),
            )
            .route(
                "/v1/graph/provenance/:node_id",
                get(graph_provenance_handler),
            )
            .route("/v1/graph/multi-hop/:node_id", get(multi_hop_graph_handler))
            // ── Memory ────────────────────────────────────────────────────────────────
            .route(
                "/v1/memories/:id/accept",
                post(accept_memory_preserved_handler),
            )
            .route(
                "/v1/memories/:id/reject",
                post(reject_memory_preserved_handler),
            )
            .route(
                "/v1/memory/provenance/:document_id",
                get(memory_provenance_handler),
            )
            // ── Providers ─────────────────────────────────────────────────────────────
            .route(
                "/v1/providers/:id/health-check",
                post(manual_provider_health_check_handler),
            )
            .route("/v1/providers/:id/recover", post(recover_provider_handler))
            .route(
                "/v1/providers/pools/:id/connections",
                post(add_pool_connection_handler),
            )
            .route(
                "/v1/providers/pools/:id/connections/:conn_id",
                delete(remove_pool_connection_handler),
            )
            .route(
                "/v1/providers/bindings/:id/cost-stats",
                get(get_binding_cost_stats_handler),
            )
            .route(
                "/v1/providers/connections/:id/models",
                get(list_provider_models_handler).post(register_provider_model_handler),
            )
            .route(
                "/v1/providers/connections/:id/health-schedule",
                get(get_provider_health_schedule_handler)
                    .post(set_provider_health_schedule_handler),
            )
            .route(
                "/v1/providers/connections/:id/retry-policy",
                put(set_provider_retry_policy_handler),
            )
            .route(
                "/v1/providers/connections/:id/resolve-key",
                get(resolve_provider_key_handler),
            )
            .route(
                "/v1/providers/connections/:id",
                put(update_provider_connection_handler).delete(delete_provider_connection_handler),
            )
            // ── Auth tokens ───────────────────────────────────────────────────────────
            .route(
                "/v1/auth/tokens",
                post(create_auth_token_handler).get(list_auth_tokens_handler),
            )
            .route("/v1/auth/tokens/:id", delete(delete_auth_token_handler))
            // ── Events + Stats ───────────────────────────────────────────────────────
            .route("/v1/events/recent", get(recent_events_handler))
            .route("/v1/stats", get(stats_handler))
            // ── Trace / Export ────────────────────────────────────────────────────────
            .route("/v1/trace/:trace_id", get(get_trace_handler))
            .route("/v1/export/:format", get(export_bundle_by_format_handler))
            .route("/healthz", get(health_handler)) // alias for k8s liveness probes
            // ── Marketplace (RFC 015) ─────────────────────────────────────────
            .route(
                "/v1/plugins/catalog",
                get(marketplace_routes::list_catalog_handler),
            )
            .route(
                "/v1/plugins/:id/install",
                post(marketplace_routes::install_plugin_handler),
            )
            .route(
                "/v1/plugins/:id/credentials",
                post(marketplace_routes::provide_credentials_handler),
            )
            .route(
                "/v1/plugins/:id/verify",
                post(marketplace_routes::verify_credentials_handler),
            )
            .route(
                "/v1/projects/:proj/plugins/:id",
                post(marketplace_routes::enable_plugin_handler)
                    .delete(marketplace_routes::disable_plugin_handler),
            )
            .route(
                "/v1/projects/:project/repos",
                get(repo_routes::list_project_repos_handler)
                    .post(repo_routes::add_project_repo_handler),
            )
            .route(
                "/v1/projects/:project/repos/:owner/:repo",
                get(repo_routes::get_project_repo_handler)
                    .delete(repo_routes::delete_project_repo_handler),
            )
            .route(
                "/v1/plugins/:id/uninstall",
                delete(marketplace_routes::uninstall_plugin_handler),
            )
            // ── Triggers (RFC 022) ────────────────────────────────────────────
            .route(
                "/v1/projects/:project/triggers",
                get(trigger_routes::list_triggers_handler)
                    .post(trigger_routes::create_trigger_handler),
            )
            .route(
                "/v1/projects/:project/triggers/:trigger_id",
                get(trigger_routes::get_trigger_handler)
                    .delete(trigger_routes::delete_trigger_handler),
            )
            .route(
                "/v1/projects/:project/triggers/:trigger_id/enable",
                post(trigger_routes::enable_trigger_handler),
            )
            .route(
                "/v1/projects/:project/triggers/:trigger_id/disable",
                post(trigger_routes::disable_trigger_handler),
            )
            .route(
                "/v1/projects/:project/triggers/:trigger_id/resume",
                post(trigger_routes::resume_trigger_handler),
            )
            // ── Run Templates (RFC 022) ───────────────────────────────────────
            .route(
                "/v1/projects/:project/run-templates",
                get(trigger_routes::list_run_templates_handler)
                    .post(trigger_routes::create_run_template_handler),
            )
            .route(
                "/v1/projects/:project/run-templates/:template_id",
                get(trigger_routes::get_run_template_handler)
                    .delete(trigger_routes::delete_run_template_handler),
            )
            // ── GitHub Webhooks & Integrations ───────────────────────────────
            .route("/v1/providers/registry", get(provider_registry_handler))
            // Legacy GitHub webhook handler — will be removed when all handlers
            // migrate to the integration registry.
            .route("/v1/webhooks/github/webhook", post(github_webhook_handler))
            .route(
                "/v1/webhooks/github/actions",
                get(list_webhook_actions_handler).put(set_webhook_actions_handler),
            )
            .route("/v1/webhooks/github/scan", post(github_scan_handler))
            .route("/v1/webhooks/github/queue", get(github_queue_handler))
            .route(
                "/v1/webhooks/github/queue/pause",
                post(github_queue_pause_handler),
            )
            .route(
                "/v1/webhooks/github/queue/resume",
                post(github_queue_resume_handler),
            )
            .route(
                "/v1/webhooks/github/queue/:issue/skip",
                post(github_queue_skip_handler),
            )
            .route(
                "/v1/webhooks/github/queue/:issue/retry",
                post(github_queue_retry_handler),
            )
            .route(
                "/v1/webhooks/github/installations",
                get(github_installations_handler),
            )
            .route(
                "/v1/webhooks/github/queue/concurrency",
                put(set_queue_concurrency_handler),
            )
            // ── Integration Plugin Registry (runtime CRUD) ─────────────────────
            .route(
                "/v1/integrations",
                get(list_integrations_handler).post(register_integration_handler),
            )
            .route(
                "/v1/integrations/:integration_id",
                get(get_integration_handler).delete(delete_integration_handler),
            )
            .route(
                "/v1/integrations/:integration_id/overrides",
                get(get_integration_overrides_handler)
                    .put(set_integration_overrides_handler)
                    .delete(clear_integration_overrides_handler),
            )
            // Dynamic webhook receiver — delegates to the registered integration.
            .route(
                "/v1/webhooks/:integration_id",
                post(dynamic_webhook_handler),
            );

        // ── RFC-011 debug endpoint ──────────────────────────────────────
        //
        // Gated behind the `debug-endpoints` Cargo feature. Absent the
        // feature, the route is not registered and requests fall through
        // to the not-found handler. See `handlers/debug.rs` for the
        // threat model; enabling this feature in production is NOT
        // supported.
        #[cfg(feature = "debug-endpoints")]
        let router = router.route(
            "/v1/admin/debug/partition",
            get(crate::handlers::debug::debug_partition_handler),
        );

        // Waitpoint HMAC rotation. Admin-only; fans out the FF 0.2
        // ff_rotate_waitpoint_hmac_secret FCALL across every
        // execution partition. Unconditionally compiled — not a debug
        // surface.
        router.route(
            "/v1/admin/rotate-waitpoint-hmac",
            post(crate::handlers::admin::rotate_waitpoint_hmac_handler),
        )
    }

    /// Apply the standard middleware stack (auth, CORS, rate-limit, tracing)
    /// to a state-resolved `Router<()>`.
    ///
    /// Call this after merging additional routes with [`build_catalog_routes`].
    pub fn apply_middleware(router: Router, state: Arc<AppState>) -> Router {
        let cors = cors_layer(&state.config);
        router
            .layer(from_fn_with_state(state.clone(), auth_middleware))
            // RFC 020 readiness gate. Positioned between auth (innermost)
            // and the CORS / request-id / observability layers so gated
            // 503 responses still pick up CORS headers (browsers need
            // them to render the "recovering" status in the SPA), an
            // `x-request-id` trace header, and Prometheus/log metrics.
            // On the request path Tower runs layers outside-in, so this
            // fires BEFORE auth: an unauthenticated probe of a
            // non-health route gets a 503 during recovery (the
            // operator-visible state) rather than a 401 (auth boundary).
            .layer(from_fn_with_state(state.clone(), readiness_middleware))
            .layer(cors)
            .layer(from_fn(request_id_middleware))
            .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
            .layer(from_fn_with_state(state.clone(), rate_limit_middleware))
            .layer(from_fn_with_state(state, observability_middleware))
    }

    /// Build the complete router: catalog routes + fallback + state + middleware.
    pub(crate) fn build_router(state: Arc<AppState>) -> Router {
        let routes = Self::build_catalog_routes()
            .fallback(not_found_handler)
            .with_state(state.clone());
        Self::apply_middleware(routes, state)
    }

    pub async fn serve_with_listener(
        &self,
        listener: TcpListener,
        config: &BootstrapConfig,
    ) -> Result<(), String> {
        let router = Self::router(config.clone()).await?;
        self.serve_with_shutdown(listener, router, std::future::pending())
            .await
    }

    /// Serve a pre-built router on the given listener. Integration-test
    /// entry point; production callers use [`Self::serve_with_listener`].
    ///
    /// Lets tests build a router via
    /// [`Self::router_with_injected_runtime`] (typically wired to a
    /// `FakeFabric` fixture) and then bind it, without going through
    /// the runtime construction inside `serve_with_listener`.
    pub async fn serve_prebuilt_router(
        &self,
        listener: TcpListener,
        router: Router,
    ) -> Result<(), String> {
        self.serve_with_shutdown(listener, router, std::future::pending())
            .await
    }

    async fn serve_with_shutdown<F>(
        &self,
        listener: TcpListener,
        router: Router,
        shutdown: F,
    ) -> Result<(), String>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|err| format!("axum server failed: {err}"))
    }

    async fn serve_with_tls_shutdown<F>(
        &self,
        addr: SocketAddr,
        router: Router,
        cert_path: &str,
        key_path: &str,
        shutdown: F,
    ) -> Result<(), String>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let tls_config = RustlsConfig::from_pem_file(cert_path, key_path)
            .await
            .map_err(|err| format!("failed to load TLS config: {err}"))?;
        let handle = AxumServerHandle::new();
        let shutdown_handle = handle.clone();
        tokio::spawn(async move {
            shutdown.await;
            shutdown_handle.graceful_shutdown(None);
        });

        axum_server::bind_rustls(addr, tls_config)
            .handle(handle)
            .serve(router.into_make_service())
            .await
            .map_err(|err| format!("axum TLS server failed: {err}"))
    }
}

impl ServerBootstrap for AppBootstrap {
    type Error = String;

    fn start(&self, config: &BootstrapConfig) -> Result<(), Self::Error> {
        let addr = config_socket_addr(config)?;
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|err| format!("failed to build tokio runtime: {err}"))?;

        if config.mode == DeploymentMode::SelfHostedTeam && !config.tls_enabled {
            tracing::warn!("TLS disabled in team mode — not recommended for production");
        }

        runtime.block_on(async {
            let router = Self::router(config.clone()).await?;
            if config.tls_enabled {
                let cert_path = config
                    .tls_cert_path
                    .as_deref()
                    .ok_or_else(|| "TLS enabled but no cert path configured".to_owned())?;
                let key_path = config
                    .tls_key_path
                    .as_deref()
                    .ok_or_else(|| "TLS enabled but no key path configured".to_owned())?;
                self.serve_with_tls_shutdown(addr, router, cert_path, key_path, shutdown_signal())
                    .await
            } else {
                let listener = TcpListener::bind(addr)
                    .await
                    .map_err(|err| format!("failed to bind {addr}: {err}"))?;
                self.serve_with_shutdown(listener, router, shutdown_signal())
                    .await
            }
        })
    }
}

// ── Handler helpers (used only by the router) ───────────────────────────────

async fn openapi_json_handler() -> impl IntoResponse {
    let mut value = match serde_json::to_value(OpenApiDoc::openapi()) {
        Ok(value) => value,
        Err(err) => {
            tracing::error!("openapi serialization failed: {err}");
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response();
        }
    };
    value["openapi"] = serde_json::Value::String("3.0.3".to_owned());
    (StatusCode::OK, Json(value)).into_response()
}

async fn swagger_docs_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <title>Cairn API Docs</title>
    <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
  </head>
  <body>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
    <script>
      window.onload = () => {
        window.ui = SwaggerUIBundle({
          url: '/openapi.json',
          dom_id: '#swagger-ui'
        });
      };
    </script>
  </body>
</html>"#,
    )
}

// ── Utility functions ───────────────────────────────────────────────────────

pub(crate) fn catalog_path_to_axum(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if let Some(param) = segment.strip_prefix(':') {
                format!("{{{param}}}")
            } else {
                segment.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn cors_layer(config: &BootstrapConfig) -> CorsLayer {
    match config.mode {
        DeploymentMode::Local => CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
        DeploymentMode::SelfHostedTeam => {
            // No allowed_origins field on BootstrapConfig; use restrictive CORS for team mode.
            CorsLayer::new()
        }
    }
}

fn config_socket_addr(config: &BootstrapConfig) -> Result<SocketAddr, String> {
    format!("{}:{}", config.listen_addr, config.listen_port)
        .parse::<SocketAddr>()
        .map_err(|_err| {
            format!(
                "invalid listen address {}:{}: {_err}",
                config.listen_addr, config.listen_port
            )
        })
}
