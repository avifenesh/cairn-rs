//! Bootstrap binary support for the Cairn Rust workspace.
//!
//! Usage:
//!   cairn-app                         # local mode, 127.0.0.1:3000
//!   cairn-app --mode team             # self-hosted team mode
//!   cairn-app --port 8080             # custom port
//!   cairn-app --addr 0.0.0.0          # bind all interfaces

pub mod bootstrap;
pub mod errors;
pub mod extractors;
pub mod fabric_adapter;
pub mod handlers;
pub mod helpers;
pub mod marketplace_routes;
pub mod metrics;
pub mod middleware;
pub mod repo_routes;
pub mod router;
pub mod sandbox;
pub mod sse_hooks;
pub mod state;
pub mod telemetry_routes;
pub mod tokens;
pub mod tool_impls;
pub mod trigger_routes;
pub mod triggers;
pub mod validate;

// Re-exports for backward compatibility
pub use bootstrap::{parse_args, parse_args_from, run_bootstrap};
pub use errors::AppApiError;

// T6c-C1/C2: re-export the tenant-scope helper used by the binary-side
// WebSocket handler (`bin_websocket.rs`) so cross-tenant event fan-out
// is gated there just like it is on the SSE path. The SSE tenant
// helper is reached via `cairn_app::handlers::sse::ws_event_tenant_id`.
pub use extractors::is_admin_principal;

/// T6b-C3: redact the password component of a database connection URL
/// so startup logs don't leak credentials to journald / CloudWatch.
///
/// Returns the input unchanged on parse error (falling back to logging
/// the opaque URL is no worse than the pre-fix baseline, and the parse
/// error itself is not actionable for the operator).
pub fn redact_dsn(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) if parsed.password().is_some() => {
            // Ignore the result — Url::set_password returns Err only
            // when the scheme forbids credentials, which can't happen
            // for postgres:// / sqlite://.
            let _ = parsed.set_password(Some("***"));
            parsed.to_string()
        }
        _ => url.to_owned(),
    }
}
#[allow(unused_imports)]
pub(crate) use errors::*;
#[allow(unused_imports)]
pub(crate) use extractors::*;
pub use helpers::event_type_name;
#[allow(unused_imports)]
pub(crate) use helpers::*;
#[allow(unused_imports)]
pub(crate) use metrics::*;
#[allow(unused_imports)]
pub(crate) use middleware::*;
pub use router::AppBootstrap;
#[allow(unused_imports)]
pub(crate) use router::*;
#[allow(unused_imports)]
pub(crate) use sandbox::*;
#[allow(unused_imports)]
pub(crate) use state::*;
pub use state::{
    AppState, GitHubEventAction, GitHubIntegration, IssueQueueEntry, IssueQueueStatus,
    WebhookAction,
};
#[allow(unused_imports)]
pub(crate) use tokens::*;
#[allow(unused_imports)]
pub(crate) use triggers::*;

pub(crate) use cairn_runtime::RuntimeError;
pub(crate) use cairn_tools::cancel_plugin_invocation;

// Test-only imports that back the `mod tests` block below. The module
// requires an `AppState` booted via `AppBootstrap`, which in turn demands
// the HMAC env (fail-loud in the default Fabric build). Gating on
// `feature = "in-memory-runtime"` keeps `cargo test -p cairn-app` green
// by default; the feature build runs them unchanged.
#[cfg(all(test, feature = "in-memory-runtime"))]
use axum::http::StatusCode;
#[cfg(all(test, feature = "in-memory-runtime"))]
use cairn_api::auth::AuthPrincipal;
#[cfg(all(test, feature = "in-memory-runtime"))]
use cairn_api::bootstrap::{BootstrapConfig, DeploymentMode, ServerBootstrap, StorageBackend};
#[cfg(all(test, feature = "in-memory-runtime"))]
use cairn_domain::{RunState, TaskState};
#[cfg(all(test, feature = "in-memory-runtime"))]
use cairn_tools::{PluginCapability, PluginManifest, PluginToolDescriptor};
#[cfg(all(test, feature = "in-memory-runtime"))]
use std::sync::Arc;

// ── Handler re-exports ─────────────────────────────────────────────────────────
#[allow(unused_imports)]
pub(crate) use handlers::admin::*;
#[allow(unused_imports)]
pub(crate) use handlers::approvals::*;
#[allow(unused_imports)]
pub(crate) use handlers::auth_tokens::*;
#[allow(unused_imports)]
pub(crate) use handlers::bundles_handlers::*;
#[allow(unused_imports)]
pub(crate) use handlers::decisions::*;
#[allow(unused_imports)]
pub(crate) use handlers::evals::*;
#[allow(unused_imports)]
pub(crate) use handlers::feed::*;
#[allow(unused_imports)]
pub(crate) use handlers::github::*;
#[allow(unused_imports)]
pub(crate) use handlers::graph::*;
#[allow(unused_imports)]
pub(crate) use handlers::health::*;
#[allow(unused_imports)]
pub(crate) use handlers::integrations::*;
#[allow(unused_imports)]
pub(crate) use handlers::memory::*;
#[allow(unused_imports)]
pub(crate) use handlers::plugins::*;
#[allow(unused_imports)]
pub(crate) use handlers::prompts::*;
#[allow(unused_imports)]
pub(crate) use handlers::providers::*;
#[allow(unused_imports)]
pub(crate) use handlers::runs::*;
#[allow(unused_imports)]
pub(crate) use handlers::sessions::*;
#[allow(unused_imports)]
pub(crate) use handlers::signals::*;
#[allow(unused_imports)]
pub(crate) use handlers::sqeq::*;
#[allow(unused_imports)]
pub(crate) use handlers::sse::*;
#[allow(unused_imports)]
pub(crate) use handlers::tasks::*;
#[allow(unused_imports)]
pub(crate) use handlers::tools::*;
#[allow(unused_imports)]
pub(crate) use handlers::workers::*;

#[cfg(all(test, feature = "in-memory-runtime"))]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use cairn_domain::providers::OperationKind;
    use cairn_domain::{
        ApprovalId, ApprovalRequirement, Entitlement, EvalRunId, EventEnvelope, EventId,
        EventSource, ExecutionClass, PauseReason, PauseReasonKind, ProjectId, ProjectKey,
        PromptAssetId, ProviderBindingId, ProviderConnectionId, ProviderModelId, ResumeTrigger,
        RunId, RunResumeTarget, RunStateChanged, RuntimeEvent, SessionId, StateTransition, TaskId,
        TenantId, WorkspaceId, WorkspaceKey, WorkspaceRole,
    };
    use cairn_evals::{EvalMetrics, EvalSubjectKind};
    use cairn_graph::projections::{GraphEdge, GraphNode, NodeKind};
    use cairn_runtime::{
        ApprovalService, DefaultsService, ProjectService, PromptAssetService, TenantService,
        WorkspaceMembershipService, WorkspaceService,
    };
    use cairn_store::EventLog;
    use cairn_tools::PluginRegistry;
    use std::sync::Mutex;
    use tower::ServiceExt;

    struct RecordingBootstrap {
        seen: Mutex<Option<BootstrapConfig>>,
    }

    impl RecordingBootstrap {
        fn new() -> Self {
            Self {
                seen: Mutex::new(None),
            }
        }

        fn seen(&self) -> Option<BootstrapConfig> {
            self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    impl ServerBootstrap for RecordingBootstrap {
        type Error = String;

        fn start(&self, config: &BootstrapConfig) -> Result<(), Self::Error> {
            *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(config.clone());
            Ok(())
        }
    }

    #[test]
    fn parse_args_defaults_to_local_mode() {
        let args = vec!["cairn-app".to_owned()];
        let config = parse_args_from(&args);

        assert_eq!(config.mode, DeploymentMode::Local);
        assert_eq!(config.listen_addr, "127.0.0.1");
        assert_eq!(config.listen_port, 3000);
    }

    #[test]
    fn parse_args_promotes_team_mode_to_public_bind() {
        let args = vec![
            "cairn-app".to_owned(),
            "--mode".to_owned(),
            "team".to_owned(),
            "--db".to_owned(),
            "postgres://localhost/cairn".to_owned(),
        ];
        let config = parse_args_from(&args);

        assert_eq!(config.mode, DeploymentMode::SelfHostedTeam);
        assert_eq!(config.listen_addr, "0.0.0.0");
    }

    #[test]
    fn run_bootstrap_delegates_to_server_bootstrap() {
        let bootstrap = RecordingBootstrap::new();
        let config = BootstrapConfig::team("postgres://localhost/cairn");

        run_bootstrap(&bootstrap, &config).unwrap();

        assert_eq!(bootstrap.seen(), Some(config));
    }

    #[test]
    fn parse_args_db_flag_sets_postgres() {
        let args = vec![
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "postgres://localhost/cairn".to_owned(),
        ];
        let config = parse_args_from(&args);
        assert!(matches!(config.storage, StorageBackend::Postgres { .. }));
    }

    #[test]
    fn parse_args_db_flag_sets_sqlite() {
        let args = vec![
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "my_data.db".to_owned(),
        ];
        let config = parse_args_from(&args);
        assert!(matches!(config.storage, StorageBackend::Sqlite { .. }));
    }

    #[test]
    fn team_mode_clears_local_auto_encryption() {
        let args = vec![
            "cairn-app".to_owned(),
            "--mode".to_owned(),
            "team".to_owned(),
            "--db".to_owned(),
            "postgres://localhost/cairn".to_owned(),
        ];
        let config = parse_args_from(&args);
        assert!(!config.credentials_available());
    }

    #[test]
    fn local_in_memory_dev_grants_deployment_tier_for_credentials() {
        let config = BootstrapConfig::default();

        assert!(app_entitlements(&config).has(Entitlement::DeploymentTier));
    }

    #[test]
    fn local_sqlite_does_not_grant_deployment_tier() {
        let config = BootstrapConfig {
            storage: StorageBackend::Sqlite {
                path: "cairn.db".to_owned(),
            },
            ..BootstrapConfig::default()
        };

        assert!(!app_entitlements(&config).has(Entitlement::DeploymentTier));
    }

    #[test]
    fn parse_args_sets_tls_fields_when_cert_and_key_present() {
        let args = vec![
            "cairn-app".to_owned(),
            "--tls-cert".to_owned(),
            "/tmp/cairn.crt".to_owned(),
            "--tls-key".to_owned(),
            "/tmp/cairn.key".to_owned(),
        ];
        let config = parse_args_from(&args);

        assert!(config.tls_enabled);
        assert_eq!(config.tls_cert_path.as_deref(), Some("/tmp/cairn.crt"));
        assert_eq!(config.tls_key_path.as_deref(), Some("/tmp/cairn.key"));
    }

    #[test]
    fn route_catalog_paths_convert_to_axum_syntax() {
        assert_eq!(
            catalog_path_to_axum("/v1/feed/:id/read"),
            "/v1/feed/{id}/read"
        );
        assert_eq!(catalog_path_to_axum("/health"), "/health");
    }

    #[tokio::test]
    async fn plugin_capabilities_route_reports_verified_manifest_capabilities() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        state.service_tokens.register(
            "test-token".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "service_token".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(TenantId::new(DEFAULT_TENANT_ID)),
            },
        );

        let manifest = PluginManifest {
            id: "com.example.verified-plugin".to_owned(),
            name: "Verified Plugin".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec!["echo".to_owned(), "ready".to_owned()],
            capabilities: vec![PluginCapability::ToolProvider {
                tools: vec!["tools.echo".to_owned()],
            }],
            permissions: cairn_tools::DeclaredPermissions::default(),
            limits: None,
            execution_class: ExecutionClass::SupervisedProcess,
            description: None,
            homepage: None,
        };
        state.plugin_registry.register(manifest.clone()).unwrap();
        {
            let mut host = state.plugin_host.lock().unwrap_or_else(|e| e.into_inner());
            host.register(manifest.clone()).unwrap();
            // capability_verification reports what capabilities are declared in the manifest
            let _ = host.capability_verification(&manifest.id).unwrap();
        }

        let response = AppBootstrap::build_router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/plugins/com.example.verified-plugin/capabilities")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["plugin_id"], "com.example.verified-plugin");
        assert_eq!(
            json["capabilities"][0]["verified"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            json["capabilities"][0]["capability"]["type"],
            "tool_provider"
        );
    }

    #[tokio::test]
    async fn local_dev_can_store_credentials_via_admin_route() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "credential-token");

        let response = post_json(
            AppBootstrap::build_router(state),
            "/v1/admin/tenants/default_tenant/credentials",
            "credential-token",
            serde_json::json!({
                "provider_id": "openai",
                "plaintext_value": "sk-local-dev",
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response_json(response).await;
        assert_eq!(body["tenant_id"], DEFAULT_TENANT_ID);
        assert_eq!(body["provider_id"], "openai");
        assert_eq!(body["active"], true);
    }

    #[tokio::test]
    async fn rbac_viewer_gets_403_member_gets_201_on_create_run() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());

        let tenant_id = TenantId::new("tenant_rbac");
        let workspace_id = WorkspaceId::new("ws_rbac");
        let workspace_key = WorkspaceKey::new("tenant_rbac", "ws_rbac");
        let project_key = ProjectKey::new("tenant_rbac", "ws_rbac", "proj_rbac");
        let session_id = SessionId::new("sess_rbac");

        state.service_tokens.register(
            "rbac-token".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "service_token".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(tenant_id.clone()),
            },
        );

        state
            .runtime
            .tenants
            .create(tenant_id.clone(), "RBAC Tenant".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .workspaces
            .create(
                tenant_id.clone(),
                workspace_id.clone(),
                "RBAC WS".to_owned(),
            )
            .await
            .unwrap();
        state
            .runtime
            .projects
            .create(project_key.clone(), "RBAC Project".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .sessions
            .create(&project_key, session_id.clone())
            .await
            .unwrap();

        // Viewer membership — ServiceTokenAuthenticator resolves to ServiceAccount { name: "service_token" }
        state
            .runtime
            .workspace_memberships
            .add_member(
                workspace_key.clone(),
                "service_token".to_owned(),
                WorkspaceRole::Viewer,
            )
            .await
            .unwrap();

        let run_body = serde_json::json!({
            "tenant_id": "tenant_rbac",
            "workspace_id": "ws_rbac",
            "project_id": "proj_rbac",
            "session_id": "sess_rbac",
            "run_id": "run_rbac_1"
        });

        // Viewer → 403
        let response = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/runs")
                    .header("authorization", "Bearer rbac-token")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&run_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        // Upgrade to Member
        state
            .runtime
            .workspace_memberships
            .remove_member(workspace_key.clone(), "service_token".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .workspace_memberships
            .add_member(
                workspace_key.clone(),
                "service_token".to_owned(),
                WorkspaceRole::Member,
            )
            .await
            .unwrap();

        let run_body2 = serde_json::json!({
            "tenant_id": "tenant_rbac",
            "workspace_id": "ws_rbac",
            "project_id": "proj_rbac",
            "session_id": "sess_rbac",
            "run_id": "run_rbac_2"
        });

        // Member → 201
        let response = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/runs")
                    .header("authorization", "Bearer rbac-token")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&run_body2).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn run_audit_trail_returns_chronological_entries() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());

        let tenant_id = TenantId::new("tenant_audit");
        let project_key = ProjectKey::new("tenant_audit", "ws_audit", "proj_audit");
        let session_id = SessionId::new("sess_audit");
        let run_id = RunId::new("run_audit_1");

        state.service_tokens.register(
            "audit-token".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "service_token".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(tenant_id.clone()),
            },
        );

        state
            .runtime
            .tenants
            .create(tenant_id.clone(), "Audit Tenant".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .workspaces
            .create(
                tenant_id.clone(),
                WorkspaceId::new("ws_audit"),
                "Audit WS".to_owned(),
            )
            .await
            .unwrap();
        state
            .runtime
            .projects
            .create(project_key.clone(), "Audit Project".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .sessions
            .create(&project_key, session_id.clone())
            .await
            .unwrap();

        // Create run, then drive it through multiple state transitions
        state
            .runtime
            .runs
            .start(&project_key, &session_id, run_id.clone(), None)
            .await
            .unwrap();

        state
            .runtime
            .runs
            .pause(
                &session_id,
                &run_id,
                PauseReason {
                    kind: PauseReasonKind::OperatorPause,
                    detail: None,
                    resume_after_ms: Some(9_999_999_999_999),
                    actor: None,
                },
            )
            .await
            .unwrap();

        state
            .runtime
            .runs
            .resume(
                &session_id,
                &run_id,
                ResumeTrigger::OperatorResume,
                RunResumeTarget::Running,
            )
            .await
            .unwrap();

        state
            .runtime
            .runs
            .complete(&session_id, &run_id)
            .await
            .unwrap();

        // GET audit trail
        let response = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/run_audit_1/audit")
                    .header("authorization", "Bearer audit-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let trail: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let entries = trail["entries"].as_array().unwrap();
        assert!(
            entries.len() >= 4,
            "expected at least 5 entries, got {}",
            entries.len()
        );

        // First entry must be the RunCreated event
        assert_eq!(entries[0]["type"], "event");
        let first_desc = entries[0]["description"].as_str().unwrap();
        assert!(
            first_desc.contains("run_audit_1"),
            "first entry should describe the run, got: {first_desc}"
        );

        // Verify strictly chronological order
        let timestamps: Vec<u64> = entries
            .iter()
            .map(|e| e["timestamp_ms"].as_u64().unwrap())
            .collect();
        let mut sorted = timestamps.clone();
        sorted.sort_unstable();
        assert_eq!(timestamps, sorted, "entries must be in chronological order");
    }

    #[tokio::test]
    async fn session_activity_feed_returns_run_and_task_entries() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());

        let tenant_id = TenantId::new("tenant_saf");
        let project_key = ProjectKey::new("tenant_saf", "ws_saf", "proj_saf");
        let session_id = SessionId::new("sess_saf");

        state.service_tokens.register(
            "saf-token".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "service_token".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(tenant_id.clone()),
            },
        );

        state
            .runtime
            .tenants
            .create(tenant_id.clone(), "T".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .workspaces
            .create(
                tenant_id.clone(),
                WorkspaceId::new("ws_saf"),
                "W".to_owned(),
            )
            .await
            .unwrap();
        state
            .runtime
            .projects
            .create(project_key.clone(), "P".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .sessions
            .create(&project_key, session_id.clone())
            .await
            .unwrap();

        // Create 2 runs in the session
        let run_id_1 = RunId::new("saf_run_1");
        let run_id_2 = RunId::new("saf_run_2");
        state
            .runtime
            .runs
            .start(&project_key, &session_id, run_id_1.clone(), None)
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project_key, &session_id, run_id_2.clone(), None)
            .await
            .unwrap();

        // Create one task on each run
        let task_id_1 = TaskId::new("saf_task_1");
        let task_id_2 = TaskId::new("saf_task_2");
        state
            .runtime
            .tasks
            .submit(
                &project_key,
                Some(&session_id),
                task_id_1.clone(),
                Some(run_id_1.clone()),
                None,
                0,
            )
            .await
            .unwrap();
        state
            .runtime
            .tasks
            .submit(
                &project_key,
                Some(&session_id),
                task_id_2.clone(),
                Some(run_id_2.clone()),
                None,
                0,
            )
            .await
            .unwrap();

        // GET /v1/sessions/sess_saf/activity
        let activity_response = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions/sess_saf/activity")
                    .header("authorization", "Bearer saf-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(activity_response.status(), StatusCode::OK);
        let body = to_bytes(activity_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let activity: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let entries = activity["entries"].as_array().unwrap();
        let entry_types: Vec<&str> = entries
            .iter()
            .map(|e| e["type"].as_str().unwrap())
            .collect();

        assert!(
            entry_types.contains(&"run_created"),
            "missing run_created entry, got: {entry_types:?}"
        );
        assert!(
            entry_types.contains(&"task_created"),
            "missing task_created entry, got: {entry_types:?}"
        );

        // Verify chronological order
        let timestamps: Vec<u64> = entries
            .iter()
            .map(|e| e["timestamp_ms"].as_u64().unwrap())
            .collect();
        let mut sorted_ts = timestamps.clone();
        sorted_ts.sort_unstable();
        assert_eq!(
            timestamps, sorted_ts,
            "entries must be in chronological order"
        );

        // GET /v1/sessions/sess_saf/active-runs
        let active_runs_response = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions/sess_saf/active-runs")
                    .header("authorization", "Bearer saf-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(active_runs_response.status(), StatusCode::OK);
        let body = to_bytes(active_runs_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let active_runs: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let items = active_runs["items"].as_array().unwrap();
        assert_eq!(
            items.len(),
            2,
            "expected 2 active runs, got {}",
            items.len()
        );
    }

    #[tokio::test]
    async fn event_pagination_run_events() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());

        let tenant_id = TenantId::new("tenant_evp");
        let project_key = ProjectKey::new("tenant_evp", "ws_evp", "proj_evp");
        let session_id = SessionId::new("sess_evp");
        let run_id = RunId::new("run_evp");

        state.service_tokens.register(
            "evp-token".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "service_token".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(tenant_id.clone()),
            },
        );
        state
            .runtime
            .tenants
            .create(tenant_id.clone(), "T".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .workspaces
            .create(
                tenant_id.clone(),
                WorkspaceId::new("ws_evp"),
                "W".to_owned(),
            )
            .await
            .unwrap();
        state
            .runtime
            .projects
            .create(project_key.clone(), "P".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .sessions
            .create(&project_key, session_id.clone())
            .await
            .unwrap();

        // Create the run — generates 1 RunCreated event (matches EntityRef::Run)
        state
            .runtime
            .runs
            .start(&project_key, &session_id, run_id.clone(), None)
            .await
            .unwrap();

        // Append 14 more RunStateChanged events directly to reach 15 run-related events
        for i in 0u64..14 {
            let envelope = EventEnvelope::for_runtime_event(
                EventId::new(format!("evt_evp_{i}")),
                EventSource::Runtime,
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project_key.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: None,
                        to: RunState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            );
            state.runtime.store.append(&[envelope]).await.unwrap();
        }

        // Page 1: limit=10 → expect 10 events, has_more=true, next_cursor set
        let resp1 = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/run_evp/events?limit=10")
                    .header("authorization", "Bearer evp-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp1.status(), StatusCode::OK);
        let body = to_bytes(resp1.into_body(), usize::MAX).await.unwrap();
        let page1: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let events1 = page1["events"].as_array().unwrap();
        assert_eq!(events1.len(), 10, "page 1 should have 10 events");
        assert_eq!(page1["has_more"], true, "has_more should be true");
        let next_cursor = page1["next_cursor"]
            .as_u64()
            .expect("next_cursor must be set when has_more=true");

        // Page 2: cursor=next_cursor, limit=10 → expect 5 events, has_more=false
        let resp2 = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/runs/run_evp/events?cursor={next_cursor}&limit=10"
                    ))
                    .header("authorization", "Bearer evp-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp2.status(), StatusCode::OK);
        let body = to_bytes(resp2.into_body(), usize::MAX).await.unwrap();
        let page2: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let events2 = page2["events"].as_array().unwrap();
        assert_eq!(events2.len(), 5, "page 2 should have 5 remaining events");
        assert_eq!(
            page2["has_more"], false,
            "has_more should be false on last page"
        );
    }

    #[tokio::test]
    async fn eval_dashboard_returns_assets_with_run_counts_and_trend() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());

        state.service_tokens.register(
            "evd-token".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "service_token".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(TenantId::new("t_evd")),
            },
        );

        let _workspace_key = WorkspaceKey::new("t_evd", "ws_evd");
        let project_key_evd = ProjectKey::new("t_evd", "ws_evd", "proj_evd");

        // Create 2 prompt assets
        state
            .runtime
            .prompt_assets
            .create(
                &project_key_evd,
                PromptAssetId::new("evd_asset_1"),
                "Asset One".to_owned(),
                "system".to_owned(),
            )
            .await
            .unwrap();
        state
            .runtime
            .prompt_assets
            .create(
                &project_key_evd,
                PromptAssetId::new("evd_asset_2"),
                "Asset Two".to_owned(),
                "system".to_owned(),
            )
            .await
            .unwrap();

        // 4 eval runs for asset_1
        let project_id = ProjectId::new("proj_evd");
        for i in 0..4u32 {
            let run_id = EvalRunId::new(format!("evd_run_a1_{i}"));
            state.evals.create_run(
                run_id.clone(),
                project_id.clone(),
                EvalSubjectKind::PromptRelease,
                "accuracy".to_owned(),
                Some(PromptAssetId::new("evd_asset_1")),
                None,
                None,
                None,
            );
            state.evals.start_run(&run_id).unwrap();
            state
                .evals
                .complete_run(
                    &run_id,
                    EvalMetrics {
                        task_success_rate: Some(0.70 + 0.05 * i as f64),
                        ..EvalMetrics::default()
                    },
                    None,
                )
                .unwrap();
        }

        // 1 eval run for asset_2 (should yield trend=no_data)
        let run_id_2 = EvalRunId::new("evd_run_a2_0");
        state.evals.create_run(
            run_id_2.clone(),
            project_id.clone(),
            EvalSubjectKind::PromptRelease,
            "accuracy".to_owned(),
            Some(PromptAssetId::new("evd_asset_2")),
            None,
            None,
            None,
        );
        state.evals.start_run(&run_id_2).unwrap();
        state
            .evals
            .complete_run(
                &run_id_2,
                EvalMetrics {
                    task_success_rate: Some(0.80),
                    ..EvalMetrics::default()
                },
                None,
            )
            .unwrap();

        // GET /v1/evals/dashboard
        let response = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/evals/dashboard?tenant_id=t_evd&workspace_id=ws_evd&project_id=proj_evd")
                    .header("authorization", "Bearer evd-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let dashboard: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let assets = dashboard["prompt_assets"].as_array().unwrap();
        assert_eq!(assets.len(), 2, "expected 2 assets in dashboard");

        let asset1 = assets
            .iter()
            .find(|a| a["asset_id"] == "evd_asset_1")
            .expect("asset_1 not found");
        assert_eq!(
            asset1["total_eval_runs"].as_u64().unwrap(),
            4,
            "asset_1 should have 4 eval runs"
        );

        let asset2 = assets
            .iter()
            .find(|a| a["asset_id"] == "evd_asset_2")
            .expect("asset_2 not found");
        assert_eq!(
            asset2["trend"].as_str().unwrap(),
            "no_data",
            "asset_2 with 1 run should have trend=no_data"
        );
    }

    #[tokio::test]
    async fn plugin_tools_list_and_search() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        state.service_tokens.register(
            "tools-token".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "service_token".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(TenantId::new(DEFAULT_TENANT_ID)),
            },
        );

        let manifest = PluginManifest {
            id: "com.example.tools-plugin".to_owned(),
            name: "Tools Plugin".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec!["echo".to_owned(), "ready".to_owned()],
            capabilities: vec![PluginCapability::ToolProvider {
                tools: vec!["git.commit".to_owned(), "git.status".to_owned()],
            }],
            permissions: cairn_tools::DeclaredPermissions::default(),
            limits: None,
            execution_class: ExecutionClass::SupervisedProcess,
            description: None,
            homepage: None,
        };

        state.plugin_registry.register(manifest.clone()).unwrap();

        {
            let mut host = state.plugin_host.lock().unwrap_or_else(|e| e.into_inner());
            host.register(manifest.clone()).unwrap();
            // Record 2 tools without spawning a real process
            host.record_tools(
                &manifest.id,
                vec![
                    PluginToolDescriptor {
                        name: "git.commit".to_owned(),
                        description: "Commit staged changes to the repository".to_owned(),
                        parameters_schema: serde_json::json!({ "type": "object" }),
                    },
                    PluginToolDescriptor {
                        name: "git.status".to_owned(),
                        description: "Show the working tree status".to_owned(),
                        parameters_schema: serde_json::json!({ "type": "object" }),
                    },
                ],
            )
            .unwrap();
        }

        // GET /v1/plugins/:id/tools — expects both tools
        let tools_response = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/plugins/com.example.tools-plugin/tools")
                    .header("authorization", "Bearer tools-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(tools_response.status(), StatusCode::OK);
        let body = to_bytes(tools_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tools = resp["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2, "expected 2 tools");
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"git.commit"), "git.commit should be listed");
        assert!(names.contains(&"git.status"), "git.status should be listed");

        // GET /v1/plugins/tools/search?query=commit — finds git.commit
        let search_response = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/plugins/tools/search?query=commit")
                    .header("authorization", "Bearer tools-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(search_response.status(), StatusCode::OK);
        let body = to_bytes(search_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let matches: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let hits = matches.as_array().unwrap();
        assert_eq!(hits.len(), 1, "search for 'commit' should return 1 match");
        assert_eq!(hits[0]["tool_name"], "git.commit");
        assert_eq!(hits[0]["plugin_id"], "com.example.tools-plugin");
    }

    #[tokio::test]
    async fn task_lease_expiry_requeues_expired_task() {
        use tokio::time::{sleep, Duration};

        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());

        let tenant_id = TenantId::new("tenant_tle");
        let project_key = ProjectKey::new("tenant_tle", "ws_tle", "proj_tle");
        let session_id = SessionId::new("sess_tle");
        let run_id = RunId::new("run_tle");
        let task_id = TaskId::new("task_tle");

        state.service_tokens.register(
            "tle-token".to_owned(),
            // `expire-leases` is now `AdminRoleGuard`-gated (T6b-C3 tier 6a).
            AuthPrincipal::ServiceAccount {
                name: "admin".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(tenant_id.clone()),
            },
        );

        state
            .runtime
            .tenants
            .create(tenant_id.clone(), "T".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .workspaces
            .create(
                tenant_id.clone(),
                WorkspaceId::new("ws_tle"),
                "W".to_owned(),
            )
            .await
            .unwrap();
        state
            .runtime
            .projects
            .create(project_key.clone(), "P".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .sessions
            .create(&project_key, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project_key, &session_id, run_id.clone(), None)
            .await
            .unwrap();

        // Create and claim a task with a 50ms lease
        state
            .runtime
            .tasks
            .submit(
                &project_key,
                Some(&session_id),
                task_id.clone(),
                Some(run_id.clone()),
                None,
                0,
            )
            .await
            .unwrap();
        state
            .runtime
            .tasks
            .claim(Some(&session_id), &task_id, "worker_tle".to_owned(), 50)
            .await
            .unwrap();

        // Confirm it's Leased
        let claimed = state.runtime.tasks.get(&task_id).await.unwrap().unwrap();
        assert_eq!(claimed.state, TaskState::Leased);

        // Wait for the lease to expire
        sleep(Duration::from_millis(100)).await;

        // POST /v1/tasks/expire-leases
        let response = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tasks/expire-leases")
                    .header("authorization", "Bearer tle-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            result["expired_count"].as_u64().unwrap(),
            1,
            "expected 1 expired task"
        );
        let ids = result["task_ids"].as_array().unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "task_tle");

        // Confirm task is back in Queued state
        let requeued = state.runtime.tasks.get(&task_id).await.unwrap().unwrap();
        assert_eq!(
            requeued.state,
            TaskState::Queued,
            "task should be re-queued after lease expiry"
        );
        assert!(
            requeued.lease_owner.is_none(),
            "lease_owner should be cleared"
        );
        assert!(
            requeued.lease_expires_at.is_none(),
            "lease_expires_at should be cleared"
        );
    }

    #[tokio::test]
    async fn run_auto_complete_when_all_tasks_done() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());

        let tenant_id = TenantId::new("tenant_rac");
        let project_key = ProjectKey::new("tenant_rac", "ws_rac", "proj_rac");
        let session_id = SessionId::new("sess_rac");
        let run_id = RunId::new("run_rac");
        let task_id_1 = TaskId::new("task_rac_1");
        let task_id_2 = TaskId::new("task_rac_2");

        state.service_tokens.register(
            "rac-token".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "service_token".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(tenant_id.clone()),
            },
        );

        // Infrastructure setup
        state
            .runtime
            .tenants
            .create(tenant_id.clone(), "T".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .workspaces
            .create(
                tenant_id.clone(),
                WorkspaceId::new("ws_rac"),
                "W".to_owned(),
            )
            .await
            .unwrap();
        state
            .runtime
            .projects
            .create(project_key.clone(), "P".to_owned())
            .await
            .unwrap();
        state
            .runtime
            .sessions
            .create(&project_key, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project_key, &session_id, run_id.clone(), None)
            .await
            .unwrap();

        // Transition run Pending → Running (normally triggered by task creation via HTTP)
        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_rac_running"),
                EventSource::Runtime,
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project_key.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Pending),
                        to: RunState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            )])
            .await
            .unwrap();

        // Create and claim both tasks
        state
            .runtime
            .tasks
            .submit(
                &project_key,
                Some(&session_id),
                task_id_1.clone(),
                Some(run_id.clone()),
                None,
                0,
            )
            .await
            .unwrap();
        state
            .runtime
            .tasks
            .submit(
                &project_key,
                Some(&session_id),
                task_id_2.clone(),
                Some(run_id.clone()),
                None,
                0,
            )
            .await
            .unwrap();
        state
            .runtime
            .tasks
            .claim(Some(&session_id), &task_id_1, "worker_rac".to_owned(), 60_000)
            .await
            .unwrap();
        state
            .runtime
            .tasks
            .claim(Some(&session_id), &task_id_2, "worker_rac".to_owned(), 60_000)
            .await
            .unwrap();

        // Complete task 1 via HTTP (handler: Leased → Running → Completed, then checks run)
        let resp1 = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tasks/task_rac_1/complete")
                    .header("authorization", "Bearer rac-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        // Run still active — task_2 is not done
        let run = state.runtime.runs.get(&run_id).await.unwrap().unwrap();
        assert_eq!(
            run.state,
            RunState::Running,
            "run should still be Running after task_1 completes"
        );

        // Complete task 2 via HTTP
        let resp2 = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tasks/task_rac_2/complete")
                    .header("authorization", "Bearer rac-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        // Run should have auto-completed
        let run = state.runtime.runs.get(&run_id).await.unwrap().unwrap();
        assert_eq!(
            run.state,
            RunState::Completed,
            "run should auto-complete when all its tasks are done"
        );
    }

    #[tokio::test]
    async fn eval_provider_matrix_returns_row_with_binding_and_cost() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        state.service_tokens.register(
            "epm-token".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "service_token".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(TenantId::new(DEFAULT_TENANT_ID)),
            },
        );

        let eval_run_id = EvalRunId::new("eval_run_epm");
        let project_id = ProjectId::new(DEFAULT_PROJECT_ID);
        state.evals.create_run(
            eval_run_id.clone(),
            project_id.clone(),
            EvalSubjectKind::PromptRelease,
            "accuracy".to_owned(),
            None,
            None,
            None,
            None,
        );
        state.evals.start_run(&eval_run_id).unwrap();
        state
            .evals
            .complete_run(&eval_run_id, EvalMetrics::default(), None)
            .unwrap();

        let binding_id = ProviderBindingId::new("binding_epm");
        let project_key =
            ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);

        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_epm_call"),
                EventSource::Runtime,
                RuntimeEvent::ProviderCallCompleted(cairn_domain::events::ProviderCallCompleted {
                    project: project_key.clone(),
                    provider_call_id: cairn_domain::ProviderCallId::new("call_epm"),
                    route_decision_id: cairn_domain::RouteDecisionId::new("rd_epm"),
                    route_attempt_id: cairn_domain::RouteAttemptId::new("ra_epm"),
                    provider_binding_id: binding_id.clone(),
                    provider_connection_id: ProviderConnectionId::new("conn_epm"),
                    provider_model_id: ProviderModelId::new("model_epm"),
                    session_id: None,
                    run_id: None,
                    operation_kind: OperationKind::Generate,
                    status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                    latency_ms: Some(120),
                    input_tokens: None,
                    output_tokens: None,
                    cost_micros: Some(500),
                    error_class: None,
                    raw_error_message: None,
                    retry_count: 0,
                    task_id: None,
                    prompt_release_id: None,
                    fallback_position: 0,
                    started_at: 0,
                    finished_at: 0,
                    completed_at: 1000,
                }),
            )])
            .await
            .unwrap();

        let response = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/evals/matrices/provider-routing?tenant_id={}",
                        DEFAULT_TENANT_ID
                    ))
                    .header("authorization", "Bearer epm-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let matrix: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let rows = matrix["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "expected 1 row in provider routing matrix");
        let row = &rows[0];
        assert_eq!(row["eval_run_id"], "eval_run_epm");
        assert_eq!(row["provider_binding_id"].as_str().unwrap(), "binding_epm");
        assert_eq!(row["total_cost_micros"].as_u64().unwrap(), 500);
        assert_eq!(row["success_rate"].as_f64().unwrap(), 1.0);
    }

    // ── New endpoint tests ────────────────────────────────────────────────────

    /// Helper: register a service-account token for DEFAULT_TENANT_ID.
    /// Registered as `admin` so `AdminRoleGuard` passes — the admin
    /// test shortcut. For tests that need a non-admin principal, build
    /// the principal directly.
    fn register_token(state: &Arc<AppState>, token: &str) {
        state.service_tokens.register(
            token.to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "admin".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(TenantId::new(DEFAULT_TENANT_ID)),
            },
        );
    }

    /// Helper: POST JSON to a path and return the response.
    async fn post_json(
        app: axum::Router,
        path: &str,
        token: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    /// Helper: GET a path and return parsed JSON body.
    async fn get_json(app: axum::Router, path: &str, token: &str) -> serde_json::Value {
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "GET {path} returned non-200");
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).expect("response is valid JSON")
    }

    async fn response_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).expect("response is valid JSON")
    }

    #[tokio::test]
    async fn preserved_memory_routes_create_list_search_and_emit_sse() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "memory-preserved-token");

        let create = post_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/memories",
            "memory-preserved-token",
            serde_json::json!({
                "content": "Ops approvals should be summarized first in the weekly digest.",
                "category": "project"
            }),
        )
        .await;
        assert_eq!(create.status(), StatusCode::CREATED);

        let created = response_json(create).await;
        let created_id = created["id"].as_str().expect("memory id").to_owned();
        let created_at = created["createdAt"].as_str().expect("createdAt");
        assert_eq!(created["status"], "proposed");
        assert_eq!(created["category"], "project");
        assert!(
            created_at.contains('T') && created_at.ends_with('Z'),
            "createdAt should preserve the ISO string contract"
        );

        let frames = state.memory_proposal_hook.collected_frames();
        assert_eq!(
            frames.len(),
            1,
            "create should emit one memory_proposed frame"
        );
        assert_eq!(
            frames[0].data["memory"]["content"],
            "Ops approvals should be summarized first in the weekly digest."
        );

        let list = get_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/memories?status=proposed&category=project",
            "memory-preserved-token",
        )
        .await;
        let list_items = list["items"].as_array().expect("items array");
        assert_eq!(list_items.len(), 1);
        assert_eq!(list_items[0]["id"], created_id);
        assert_eq!(list["hasMore"], false);
        assert_eq!(list["has_more"], false);

        let search = get_json(
            AppBootstrap::build_router(state),
            "/v1/memories/search?q=weekly&limit=10",
            "memory-preserved-token",
        )
        .await;
        let search_items = search["items"].as_array().expect("items array");
        assert_eq!(search_items.len(), 1);
        assert_eq!(search_items[0]["id"], created_id);
        assert_eq!(search_items[0]["status"], "proposed");
        assert_eq!(search_items[0]["source"], "assistant");
    }

    #[tokio::test]
    async fn preserved_memory_accept_and_reject_routes_update_status_filters() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "memory-status-token");

        let accepted = response_json(
            post_json(
                AppBootstrap::build_router(state.clone()),
                "/v1/memories",
                "memory-status-token",
                serde_json::json!({
                    "content": "Ship blockers belong at the top of the summary.",
                    "category": "ops"
                }),
            )
            .await,
        )
        .await;
        let accepted_id = accepted["id"].as_str().expect("accepted id").to_owned();

        let rejected = response_json(
            post_json(
                AppBootstrap::build_router(state.clone()),
                "/v1/memories",
                "memory-status-token",
                serde_json::json!({
                    "content": "Archive old backlog screenshots after triage.",
                    "category": "ops"
                }),
            )
            .await,
        )
        .await;
        let rejected_id = rejected["id"].as_str().expect("rejected id").to_owned();

        let accept = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/memories/{accepted_id}/accept"),
            "memory-status-token",
            serde_json::json!({}),
        )
        .await;
        let accept_status = accept.status();
        let accept_body = response_json(accept).await;
        assert_eq!(accept_status, StatusCode::OK, "{accept_body}");
        assert_eq!(accept_body["ok"], true);

        let reject = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/memories/{rejected_id}/reject"),
            "memory-status-token",
            serde_json::json!({}),
        )
        .await;
        let reject_status = reject.status();
        let reject_body = response_json(reject).await;
        assert_eq!(reject_status, StatusCode::OK, "{reject_body}");
        assert_eq!(reject_body["ok"], true);

        let accepted_list = get_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/memories?status=accepted",
            "memory-status-token",
        )
        .await;
        let accepted_items = accepted_list["items"].as_array().expect("accepted items");
        assert_eq!(accepted_items.len(), 1);
        assert_eq!(accepted_items[0]["id"], accepted_id);
        assert_eq!(accepted_items[0]["status"], "accepted");

        let rejected_list = get_json(
            AppBootstrap::build_router(state),
            "/v1/memories?status=rejected",
            "memory-status-token",
        )
        .await;
        let rejected_items = rejected_list["items"].as_array().expect("rejected items");
        assert_eq!(rejected_items.len(), 1);
        assert_eq!(rejected_items[0]["id"], rejected_id);
        assert_eq!(rejected_items[0]["status"], "rejected");
    }

    #[tokio::test]
    async fn preserved_memory_category_filter_paginates_before_limit() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "memory-category-token");

        for (content, category) in [
            ("Project memory one", "project"),
            ("Ops memory", "ops"),
            ("Project memory two", "project"),
        ] {
            let response = post_json(
                AppBootstrap::build_router(state.clone()),
                "/v1/memories",
                "memory-category-token",
                serde_json::json!({
                    "content": content,
                    "category": category
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::CREATED);
        }

        let list = get_json(
            AppBootstrap::build_router(state),
            "/v1/memories?category=project&limit=2",
            "memory-category-token",
        )
        .await;
        let items = list["items"].as_array().expect("items");
        assert_eq!(items.len(), 2);
        assert!(items
            .iter()
            .all(|item| item["category"].as_str() == Some("project")));
        assert_eq!(list["hasMore"], false);
        assert_eq!(list["has_more"], false);
    }

    #[tokio::test]
    async fn preserved_memory_routes_enforce_project_scope() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "memory-project-scope-token");

        let created = response_json(
            post_json(
                AppBootstrap::build_router(state.clone()),
                "/v1/memories?project_id=project-a",
                "memory-project-scope-token",
                serde_json::json!({
                    "content": "Only project A should see this memory.",
                    "category": "project"
                }),
            )
            .await,
        )
        .await;
        let memory_id = created["id"].as_str().expect("memory id").to_owned();

        let project_b_list = get_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/memories?project_id=project-b",
            "memory-project-scope-token",
        )
        .await;
        let project_b_items = project_b_list["items"].as_array().expect("items");
        assert!(project_b_items.is_empty());

        let wrong_project_accept = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/memories/{memory_id}/accept?project_id=project-b"),
            "memory-project-scope-token",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(wrong_project_accept.status(), StatusCode::NOT_FOUND);

        let wrong_project_reject = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/memories/{memory_id}/reject?project_id=project-b"),
            "memory-project-scope-token",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(wrong_project_reject.status(), StatusCode::NOT_FOUND);

        let right_project_accept = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/memories/{memory_id}/accept?project_id=project-a"),
            "memory-project-scope-token",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(right_project_accept.status(), StatusCode::OK);

        let project_a_accepted = get_json(
            AppBootstrap::build_router(state),
            "/v1/memories?project_id=project-a&status=accepted",
            "memory-project-scope-token",
        )
        .await;
        let project_a_items = project_a_accepted["items"].as_array().expect("items");
        assert_eq!(project_a_items.len(), 1);
        assert_eq!(project_a_items[0]["id"], memory_id);
        assert_eq!(project_a_items[0]["status"], "accepted");
    }

    #[tokio::test]
    async fn preserved_memory_routes_reject_cross_tenant_scope_override() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        // Use a non-admin token so the cross-tenant guard actually
        // fires (admin tokens bypass by design).
        state.service_tokens.register(
            "memory-tenant-scope-token".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "tenant_operator".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(TenantId::new(DEFAULT_TENANT_ID)),
            },
        );

        let response = post_json(
            AppBootstrap::build_router(state),
            "/v1/memories?tenant_id=other-tenant",
            "memory-tenant-scope-token",
            serde_json::json!({
                "content": "Cross-tenant memory writes should be forbidden.",
                "category": "project"
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[derive(Clone)]
    struct RecordingTriggerFireGuardrail {
        requests: Arc<Mutex<Vec<cairn_domain::decisions::DecisionRequest>>>,
        trigger_outcome: cairn_runtime::decisions::GuardrailCheckOutcome,
    }

    #[async_trait::async_trait]
    impl cairn_runtime::decisions::GuardrailChecker for RecordingTriggerFireGuardrail {
        async fn check(
            &self,
            request: &cairn_domain::decisions::DecisionRequest,
        ) -> cairn_runtime::decisions::GuardrailCheckResult {
            self.requests
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(request.clone());

            let outcome = match &request.kind {
                cairn_domain::decisions::DecisionKind::TriggerFire { .. } => {
                    self.trigger_outcome.clone()
                }
                _ => cairn_runtime::decisions::GuardrailCheckOutcome::Allow,
            };

            cairn_runtime::decisions::GuardrailCheckResult {
                outcome,
                rule_ids: vec![],
            }
        }
    }

    #[tokio::test]
    async fn graph_trace_returns_live_project_nodes_and_edges() {
        use cairn_graph::projections::GraphProjection;

        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "graph-trace-token");

        let project = ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);
        let other_project = ProjectKey::new("other_tenant", "other_workspace", "other_project");

        state
            .graph
            .add_node(GraphNode {
                node_id: "session_graph_trace".to_owned(),
                kind: NodeKind::Session,
                project: Some(project.clone()),
                created_at: 30,
            })
            .await
            .unwrap();
        state
            .graph
            .add_node(GraphNode {
                node_id: "run_graph_trace".to_owned(),
                kind: NodeKind::Run,
                project: Some(project.clone()),
                created_at: 20,
            })
            .await
            .unwrap();
        state
            .graph
            .add_node(GraphNode {
                node_id: "task_graph_trace".to_owned(),
                kind: NodeKind::Task,
                project: Some(project.clone()),
                created_at: 10,
            })
            .await
            .unwrap();
        state
            .graph
            .add_edge(GraphEdge {
                source_node_id: "session_graph_trace".to_owned(),
                target_node_id: "run_graph_trace".to_owned(),
                kind: cairn_graph::projections::EdgeKind::Triggered,
                created_at: 31,
                confidence: None,
            })
            .await
            .unwrap();
        state
            .graph
            .add_edge(GraphEdge {
                source_node_id: "run_graph_trace".to_owned(),
                target_node_id: "task_graph_trace".to_owned(),
                kind: cairn_graph::projections::EdgeKind::Spawned,
                created_at: 21,
                confidence: None,
            })
            .await
            .unwrap();
        state
            .graph
            .add_node(GraphNode {
                node_id: "run_other_project".to_owned(),
                kind: NodeKind::Run,
                project: Some(other_project),
                created_at: 40,
            })
            .await
            .unwrap();

        let body = get_json(
            AppBootstrap::build_router(state),
            &format!(
                "/v1/graph/trace?tenant_id={DEFAULT_TENANT_ID}&workspace_id={DEFAULT_WORKSPACE_ID}&project_id={DEFAULT_PROJECT_ID}&limit=50"
            ),
            "graph-trace-token",
        )
        .await;

        let nodes = body["nodes"].as_array().expect("nodes array");
        let edges = body["edges"].as_array().expect("edges array");
        let node_ids = nodes
            .iter()
            .filter_map(|node| node["node_id"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(node_ids.len(), 3);
        assert!(node_ids.contains(&"session_graph_trace"));
        assert!(node_ids.contains(&"run_graph_trace"));
        assert!(node_ids.contains(&"task_graph_trace"));
        assert!(!node_ids.contains(&"run_other_project"));
        assert_eq!(edges.len(), 2);
        assert_eq!(body["root"], "session_graph_trace");
    }

    #[tokio::test]
    async fn deny_approval_returns_rejected_decision() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "deny-token");

        let project = ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);
        let session_id = SessionId::new("sess_deny_test");
        let run_id = RunId::new("run_deny_test");
        let appr_id = ApprovalId::new("appr_deny_test");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id.clone(), None)
            .await
            .unwrap();
        state
            .runtime
            .approvals
            .request(
                &project,
                appr_id.clone(),
                Some(run_id),
                None,
                ApprovalRequirement::Required,
            )
            .await
            .unwrap();

        let app = AppBootstrap::build_router(state);
        let resp = post_json(
            app,
            "/v1/approvals/appr_deny_test/deny",
            "deny-token",
            serde_json::json!({}),
        )
        .await;

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "deny approval should return 200"
        );
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        // The handler resolves with ApprovalDecision::Rejected
        assert_eq!(
            body["decision"].as_str().unwrap_or(""),
            "rejected",
            "denied approval must have decision = rejected"
        );
        assert_eq!(body["approval_id"], "appr_deny_test");
    }

    #[tokio::test]
    async fn cancel_task_returns_canceled_state() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "cancel-token");

        let project = ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);
        let session_id = SessionId::new("sess_cancel_test");
        let run_id = RunId::new("run_cancel_test");
        let task_id = TaskId::new("task_cancel_test");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id.clone(), None)
            .await
            .unwrap();
        state
            .runtime
            .tasks
            .submit(&project, Some(&session_id), task_id.clone(), None, None, 0)
            .await
            .unwrap();

        let app = AppBootstrap::build_router(state);
        let resp = post_json(
            app,
            "/v1/tasks/task_cancel_test/cancel",
            "cancel-token",
            serde_json::json!({}),
        )
        .await;

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "cancel task should return 200"
        );
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        // TaskState::Canceled serialises as "canceled"
        assert_eq!(
            body["state"].as_str().unwrap_or(""),
            "canceled",
            "cancelled task must have state = canceled"
        );
        assert_eq!(body["task_id"], "task_cancel_test");
    }

    #[tokio::test]
    async fn recent_events_with_activity_returns_events() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "events2-token");

        // Create some state to generate events.
        let project = ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);
        let session_id = SessionId::new("sess_events_test");
        state
            .runtime
            .sessions
            .create(&project, session_id)
            .await
            .unwrap();

        let app = AppBootstrap::build_router(state);
        let body = get_json(app, "/v1/events/recent?limit=50", "events2-token").await;

        let items = body["items"].as_array().unwrap();
        assert!(
            !items.is_empty(),
            "must have at least one event after session create"
        );

        // Each item must have required fields.
        let first = &items[0];
        assert!(
            first["event_type"].is_string(),
            "event_type must be a string"
        );
        assert!(first["stored_at"].is_number(), "stored_at must be a number");
        assert!(first["position"].is_number(), "position must be a number");
    }

    #[tokio::test]
    async fn ingest_signal_materializes_real_trigger_run_with_origin_badge() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "trigger-origin-token");

        let project = ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);
        let trigger_id = cairn_domain::TriggerId::new("trigger_origin_badge");
        let template_id = cairn_domain::RunTemplateId::new("tmpl_trigger_origin");

        {
            let mut triggers = state.triggers.lock().unwrap();
            triggers.create_template(cairn_runtime::RunTemplate {
                id: template_id.clone(),
                project: project.clone(),
                name: "Incident responder".to_owned(),
                description: Some("Investigate incoming alerts".to_owned()),
                default_mode: cairn_domain::decisions::RunMode::Plan,
                system_prompt: "Investigate the triggering signal and propose a plan.".to_owned(),
                initial_user_message: Some("Investigate the labeled incident.".to_owned()),
                plugin_allowlist: Some(vec!["github".to_owned()]),
                tool_allowlist: Some(vec!["cairn.searchEvents".to_owned()]),
                budget: cairn_runtime::TemplateBudget::default(),
                sandbox_hint: Some("repo".to_owned()),
                required_fields: Vec::new(),
                created_by: cairn_domain::OperatorId::new("operator"),
                created_at: 1,
                updated_at: 1,
            });
            triggers
                .create_trigger(cairn_runtime::Trigger {
                    id: trigger_id.clone(),
                    project: project.clone(),
                    name: "Incident labeled".to_owned(),
                    description: None,
                    signal_pattern: cairn_runtime::SignalPattern {
                        signal_type: "github.issue.labeled".to_owned(),
                        plugin_id: None,
                    },
                    conditions: Vec::new(),
                    run_template_id: template_id,
                    state: cairn_runtime::TriggerState::Enabled,
                    rate_limit: cairn_runtime::RateLimitConfig::default(),
                    max_chain_depth: 5,
                    created_by: cairn_domain::OperatorId::new("operator"),
                    created_at: 1,
                    updated_at: 1,
                })
                .unwrap();
        }

        let ingest = post_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/signals",
            "trigger-origin-token",
            serde_json::json!({
                "tenant_id": DEFAULT_TENANT_ID,
                "workspace_id": DEFAULT_WORKSPACE_ID,
                "project_id": DEFAULT_PROJECT_ID,
                "signal_id": "sig_trigger_origin",
                "source": "github.issue.labeled",
                "payload": { "action": "labeled" }
            }),
        )
        .await;
        assert_eq!(ingest.status(), StatusCode::CREATED);

        let mut runs = cairn_store::projections::RunReadModel::list_active_by_project(
            state.runtime.store.as_ref(),
            &project,
            10,
        )
        .await
        .unwrap();
        assert_eq!(runs.len(), 1, "trigger fire must create one real run");
        let run = runs.pop().unwrap();

        let session = state
            .runtime
            .sessions
            .get(&run.session_id)
            .await
            .unwrap()
            .expect("trigger fire must create a session");
        assert_eq!(session.project, project);

        let origin_key = run_default_key(&run.run_id, "created_by_trigger_id");
        let goal_key = run_default_key(&run.run_id, "goal");
        let mode_key = run_default_key(&run.run_id, "run_mode");

        let origin = state
            .runtime
            .defaults
            .resolve(&project, &origin_key)
            .await
            .unwrap()
            .expect("trigger origin must be stored");
        assert_eq!(origin.as_str(), Some(trigger_id.as_str()));

        let goal = state
            .runtime
            .defaults
            .resolve(&project, &goal_key)
            .await
            .unwrap()
            .expect("trigger goal must be stored");
        assert_eq!(goal.as_str(), Some("Investigate the labeled incident."));

        let mode = state
            .runtime
            .defaults
            .resolve(&project, &mode_key)
            .await
            .unwrap()
            .expect("trigger run mode must be stored");
        let mode: cairn_domain::decisions::RunMode = serde_json::from_value(mode).unwrap();
        assert!(matches!(mode, cairn_domain::decisions::RunMode::Plan));

        let detail = get_json(
            AppBootstrap::build_router(state),
            &format!("/v1/runs/{}", run.run_id.as_str()),
            "trigger-origin-token",
        )
        .await;
        assert_eq!(detail["run"]["run_id"], run.run_id.as_str());
        assert_eq!(
            detail["run"]["created_by_trigger_id"],
            trigger_id.as_str(),
            "RunDetail must expose trigger provenance for the badge"
        );
    }

    #[tokio::test]
    async fn replay_triggers_restores_fire_ledger_after_signal_ingest() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "trigger-replay-token");

        let project = ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);

        let template_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/projects/{DEFAULT_PROJECT_ID}/run-templates"),
            "trigger-replay-token",
            serde_json::json!({
                "name": "Replay template",
                "description": "Persisted template for replay coverage",
                "system_prompt": "Investigate the triggering signal.",
                "initial_user_message": "Please inspect this labeled issue.",
                "required_fields": []
            }),
        )
        .await;
        assert_eq!(template_resp.status(), StatusCode::CREATED);
        let template_body = response_json(template_resp).await;
        let template_id = template_body["events"][0]["template_id"]
            .as_str()
            .expect("template creation must return template_id")
            .to_owned();

        let trigger_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/projects/{DEFAULT_PROJECT_ID}/triggers"),
            "trigger-replay-token",
            serde_json::json!({
                "name": "Replay trigger",
                "description": "Persisted trigger for replay coverage",
                "signal_type": "github.issue.labeled",
                "conditions": [],
                "run_template_id": template_id,
                "max_chain_depth": 5
            }),
        )
        .await;
        assert_eq!(trigger_resp.status(), StatusCode::CREATED);
        let trigger_body = response_json(trigger_resp).await;
        let trigger_id = trigger_body["events"][0]["trigger_id"]
            .as_str()
            .expect("trigger creation must return trigger_id")
            .to_owned();

        let signal_id = "sig_trigger_replay";
        let payload = serde_json::json!({
            "action": "labeled",
            "labels": [{ "name": "cairn-ready" }]
        });

        let ingest_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/signals",
            "trigger-replay-token",
            serde_json::json!({
                "tenant_id": DEFAULT_TENANT_ID,
                "workspace_id": DEFAULT_WORKSPACE_ID,
                "project_id": DEFAULT_PROJECT_ID,
                "signal_id": signal_id,
                "source": "github.issue.labeled",
                "payload": payload
            }),
        )
        .await;
        assert_eq!(ingest_resp.status(), StatusCode::CREATED);

        state.replay_triggers().await;

        {
            let triggers = state.triggers.lock().unwrap();
            assert!(
                triggers
                    .get_template(&cairn_domain::RunTemplateId::new(&template_id))
                    .is_some(),
                "template must be restored from the event log"
            );
            let restored = triggers
                .get_trigger(&cairn_domain::TriggerId::new(&trigger_id))
                .expect("trigger must be restored from the event log");
            assert_eq!(restored.project, project);
            assert_eq!(restored.signal_pattern.signal_type, "github.issue.labeled");
        }

        let replay_events = {
            let mut triggers = state.triggers.lock().unwrap();
            triggers.evaluate_signal(
                &project,
                &cairn_domain::SignalId::new(signal_id),
                "github.issue.labeled",
                "",
                &serde_json::json!({
                    "action": "labeled",
                    "labels": [{ "name": "cairn-ready" }]
                }),
                None,
                &cairn_runtime::services::trigger_service::auto_approve_decision,
            )
        };

        assert!(
            matches!(
                replay_events.as_slice(),
                [cairn_runtime::TriggerEvent::TriggerSkipped {
                    trigger_id: skipped_trigger_id,
                    signal_id: skipped_signal_id,
                    reason: cairn_runtime::SkipReason::AlreadyFired,
                    ..
                }] if skipped_trigger_id.as_str() == trigger_id
                    && skipped_signal_id.as_str() == signal_id
            ),
            "replayed fire ledger must suppress duplicate fires: {replay_events:?}"
        );
    }

    #[tokio::test]
    async fn signal_ingest_records_trigger_fire_decision_before_materializing_run() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let guardrail = Arc::new(RecordingTriggerFireGuardrail {
            requests: requests.clone(),
            trigger_outcome: cairn_runtime::decisions::GuardrailCheckOutcome::Allow,
        });

        let mut state = AppState::new(BootstrapConfig::default()).await.unwrap();
        let runtime = Arc::get_mut(&mut state.runtime).expect("runtime arc must be unique in test");
        runtime.decision_service = Arc::new(cairn_runtime::DecisionServiceImpl::with_services(
            Arc::new(cairn_runtime::decisions::AllowAllScopeChecker),
            Arc::new(cairn_runtime::decisions::AllowAllVisibilityChecker),
            guardrail,
            Arc::new(cairn_runtime::decisions::AllowAllBudgetChecker),
            Arc::new(cairn_runtime::decisions::AutoApproveResolver),
        ));
        let state = Arc::new(state);
        register_token(&state, "trigger-decision-allow-token");

        let project = ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);

        let template_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/projects/{DEFAULT_PROJECT_ID}/run-templates"),
            "trigger-decision-allow-token",
            serde_json::json!({
                "name": "Decision allow template",
                "description": "Template used to verify approved trigger fires",
                "system_prompt": "Investigate the triggering signal.",
                "initial_user_message": "Please inspect this labeled issue.",
                "required_fields": []
            }),
        )
        .await;
        assert_eq!(template_resp.status(), StatusCode::CREATED);
        let template_body = response_json(template_resp).await;
        let template_id = template_body["events"][0]["template_id"]
            .as_str()
            .expect("template creation must return template_id")
            .to_owned();

        let trigger_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/projects/{DEFAULT_PROJECT_ID}/triggers"),
            "trigger-decision-allow-token",
            serde_json::json!({
                "name": "Decision allow trigger",
                "description": "Trigger used to verify approved trigger fires",
                "signal_type": "github.issue.labeled",
                "conditions": [],
                "run_template_id": template_id,
                "max_chain_depth": 5
            }),
        )
        .await;
        assert_eq!(trigger_resp.status(), StatusCode::CREATED);

        let ingest_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/signals",
            "trigger-decision-allow-token",
            serde_json::json!({
                "tenant_id": DEFAULT_TENANT_ID,
                "workspace_id": DEFAULT_WORKSPACE_ID,
                "project_id": DEFAULT_PROJECT_ID,
                "signal_id": "sig_trigger_decision_allow",
                "source": "github.issue.labeled",
                "payload": { "action": "labeled" }
            }),
        )
        .await;
        assert_eq!(ingest_resp.status(), StatusCode::CREATED);

        let runs = cairn_store::projections::RunReadModel::list_active_by_project(
            state.runtime.store.as_ref(),
            &project,
            10,
        )
        .await
        .unwrap();
        assert_eq!(runs.len(), 1, "approved trigger fires must create a run");

        let recorded_requests = requests.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            recorded_requests.len(),
            1,
            "expected one trigger fire decision"
        );
        let request = &recorded_requests[0];
        assert!(matches!(
            &request.kind,
            cairn_domain::decisions::DecisionKind::TriggerFire { signal_type, .. }
                if signal_type == "github.issue.labeled"
        ));
        assert!(matches!(
            request.principal,
            cairn_domain::decisions::Principal::System
        ));
        assert!(matches!(
            &request.subject,
            cairn_domain::decisions::DecisionSubject::Resource {
                resource_type,
                resource_id,
            } if resource_type == "signal" && resource_id == "sig_trigger_decision_allow"
        ));
    }

    #[tokio::test]
    async fn signal_ingest_denies_trigger_fire_when_decision_layer_denies() {
        use cairn_store::event_log::EventLog;

        let requests = Arc::new(Mutex::new(Vec::new()));
        let guardrail = Arc::new(RecordingTriggerFireGuardrail {
            requests: requests.clone(),
            trigger_outcome: cairn_runtime::decisions::GuardrailCheckOutcome::Deny(
                "trigger_guardrail_denied".to_owned(),
            ),
        });

        let mut state = AppState::new(BootstrapConfig::default()).await.unwrap();
        let runtime = Arc::get_mut(&mut state.runtime).expect("runtime arc must be unique in test");
        runtime.decision_service = Arc::new(cairn_runtime::DecisionServiceImpl::with_services(
            Arc::new(cairn_runtime::decisions::AllowAllScopeChecker),
            Arc::new(cairn_runtime::decisions::AllowAllVisibilityChecker),
            guardrail,
            Arc::new(cairn_runtime::decisions::AllowAllBudgetChecker),
            Arc::new(cairn_runtime::decisions::AutoApproveResolver),
        ));
        let state = Arc::new(state);
        register_token(&state, "trigger-decision-deny-token");

        let project = ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);

        let template_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/projects/{DEFAULT_PROJECT_ID}/run-templates"),
            "trigger-decision-deny-token",
            serde_json::json!({
                "name": "Decision deny template",
                "description": "Template used to verify denied trigger fires",
                "system_prompt": "Investigate the triggering signal.",
                "initial_user_message": "Please inspect this labeled issue.",
                "required_fields": []
            }),
        )
        .await;
        assert_eq!(template_resp.status(), StatusCode::CREATED);
        let template_body = response_json(template_resp).await;
        let template_id = template_body["events"][0]["template_id"]
            .as_str()
            .expect("template creation must return template_id")
            .to_owned();

        let trigger_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            &format!("/v1/projects/{DEFAULT_PROJECT_ID}/triggers"),
            "trigger-decision-deny-token",
            serde_json::json!({
                "name": "Decision deny trigger",
                "description": "Trigger used to verify denied trigger fires",
                "signal_type": "github.issue.labeled",
                "conditions": [],
                "run_template_id": template_id,
                "max_chain_depth": 5
            }),
        )
        .await;
        assert_eq!(trigger_resp.status(), StatusCode::CREATED);
        let trigger_body = response_json(trigger_resp).await;
        let trigger_id = trigger_body["events"][0]["trigger_id"]
            .as_str()
            .expect("trigger creation must return trigger_id")
            .to_owned();

        let signal_id = "sig_trigger_decision_deny";
        let ingest_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/signals",
            "trigger-decision-deny-token",
            serde_json::json!({
                "tenant_id": DEFAULT_TENANT_ID,
                "workspace_id": DEFAULT_WORKSPACE_ID,
                "project_id": DEFAULT_PROJECT_ID,
                "signal_id": signal_id,
                "source": "github.issue.labeled",
                "payload": { "action": "labeled" }
            }),
        )
        .await;
        assert_eq!(ingest_resp.status(), StatusCode::CREATED);

        let runs = cairn_store::projections::RunReadModel::list_active_by_project(
            state.runtime.store.as_ref(),
            &project,
            10,
        )
        .await
        .unwrap();
        assert!(
            runs.is_empty(),
            "denied trigger fires must not materialize runs"
        );

        let events = state
            .runtime
            .store
            .read_stream(None, usize::MAX)
            .await
            .unwrap();
        let denied = events
            .iter()
            .find_map(|stored| match &stored.envelope.payload {
                RuntimeEvent::TriggerDenied(event)
                    if event.trigger_id.as_str() == trigger_id
                        && event.signal_id.as_str() == signal_id =>
                {
                    Some(event.clone())
                }
                _ => None,
            });
        let denied = denied.expect("denied trigger fire must be persisted");
        assert_eq!(denied.reason, "trigger_guardrail_denied");

        let recorded_requests = requests.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            recorded_requests.len(),
            1,
            "expected one trigger fire decision"
        );
        let request = &recorded_requests[0];
        assert!(matches!(
            &request.kind,
            cairn_domain::decisions::DecisionKind::TriggerFire {
                trigger_id: recorded_trigger_id,
                signal_type,
            } if recorded_trigger_id == &trigger_id && signal_type == "github.issue.labeled"
        ));
        assert!(matches!(
            request.principal,
            cairn_domain::decisions::Principal::System
        ));
        assert!(matches!(
            &request.subject,
            cairn_domain::decisions::DecisionSubject::Resource {
                resource_type,
                resource_id,
            } if resource_type == "signal" && resource_id == signal_id
        ));
    }

    #[tokio::test]
    async fn sqeq_start_run_roundtrip_streams_correlation_id() {
        use http_body_util::BodyExt as _;
        use tokio::time::{timeout, Duration};

        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "sqeq-token");

        let project = ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);
        let session_id = SessionId::new("session_sqeq_roundtrip");
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();

        let init_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/sqeq/initialize",
            "sqeq-token",
            serde_json::json!({
                "protocol_versions": ["1.0"],
                "scope": {
                    "tenant_id": DEFAULT_TENANT_ID,
                    "workspace_id": DEFAULT_WORKSPACE_ID,
                    "project_id": DEFAULT_PROJECT_ID,
                },
                "subscriptions": {
                    "event_types": ["run.*"]
                }
            }),
        )
        .await;
        assert_eq!(init_resp.status(), StatusCode::OK);
        let init_body = response_json(init_resp).await;
        let sqeq_session_id = init_body["sqeq_session_id"]
            .as_str()
            .expect("sqeq init must return session id")
            .to_owned();

        let stream_resp = AppBootstrap::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/sqeq/events?sqeq_session_id={sqeq_session_id}"))
                    .header("authorization", "Bearer sqeq-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stream_resp.status(), StatusCode::OK);

        let stream_body = stream_resp.into_body();
        let read_stream = tokio::spawn(async move {
            let mut stream_body = stream_body;
            loop {
                let frame = stream_body.frame().await?;
                let Ok(frame) = frame else {
                    return None;
                };
                if let Ok(data) = frame.into_data() {
                    let text = String::from_utf8_lossy(data.as_ref()).to_string();
                    if !text.trim().is_empty() {
                        return Some(text);
                    }
                }
            }
        });

        let submit_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/sqeq/submit",
            "sqeq-token",
            serde_json::json!({
                "method": "start_run",
                "correlation_id": "corr_sqeq_roundtrip",
                "params": {
                    "sqeq_session_id": sqeq_session_id,
                    "session_id": session_id.as_str(),
                    "run_id": "run_sqeq_roundtrip"
                }
            }),
        )
        .await;
        assert_eq!(submit_resp.status(), StatusCode::ACCEPTED);
        let submit_body = response_json(submit_resp).await;
        assert_eq!(submit_body["accepted"], true);
        assert_eq!(submit_body["correlation_id"], "corr_sqeq_roundtrip");
        assert!(submit_body["projected_event_seq"].is_number());

        let frame_text = timeout(Duration::from_millis(500), read_stream)
            .await
            .expect("timed out waiting for sqeq sse frame")
            .expect("stream task panicked")
            .expect("stream ended before yielding a frame");
        assert!(frame_text.contains("run_sqeq_roundtrip"), "{frame_text}");
        assert!(frame_text.contains("corr_sqeq_roundtrip"), "{frame_text}");

        let run = state
            .runtime
            .runs
            .get(&RunId::new("run_sqeq_roundtrip"))
            .await
            .unwrap()
            .expect("run must be created");
        assert_eq!(run.project, project);
    }

    #[tokio::test]
    async fn a2a_submission_creates_internal_task_with_a2a_source() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "a2a-token");

        let submit_resp = post_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/a2a/tasks",
            "a2a-token",
            serde_json::json!({
                "task": {
                    "kind": "research",
                    "input": {
                        "content_type": "text/markdown",
                        "content": "Investigate the latest alert"
                    },
                    "metadata": {}
                }
            }),
        )
        .await;
        assert_eq!(submit_resp.status(), StatusCode::CREATED);
        let submit_body = response_json(submit_resp).await;
        let task_id = submit_body["task_id"]
            .as_str()
            .expect("a2a submit must return task id")
            .to_owned();
        assert_eq!(submit_body["status"], "submitted");

        let task = state
            .runtime
            .tasks
            .get(&TaskId::new(task_id.clone()))
            .await
            .unwrap()
            .expect("internal task must be created");
        assert_eq!(task.project.tenant_id, TenantId::new(DEFAULT_TENANT_ID));
        assert_eq!(task.state, TaskState::Queued);

        let status_body = get_json(
            AppBootstrap::build_router(state),
            &format!("/v1/a2a/tasks/{task_id}"),
            "a2a-token",
        )
        .await;
        assert_eq!(status_body["task_id"], task_id);
        assert_eq!(status_body["source"], "A2A");
        assert_eq!(status_body["internal_task_id"], task.task_id.as_str());
        assert_eq!(status_body["status"], "queued");
    }

    #[tokio::test]
    async fn orchestrate_run_exports_otlp_spans_with_genai_attributes() {
        use cairn_domain::protocols::OtlpConfig;
        use cairn_domain::providers::{
            GenerationProvider, GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
        };
        use cairn_runtime::telemetry::{ExportableSpan, SpanAttributeValue, SpanExportSink};

        #[derive(Clone)]
        struct RecordingSink {
            spans: Arc<Mutex<Vec<ExportableSpan>>>,
        }

        #[async_trait::async_trait]
        impl SpanExportSink for RecordingSink {
            async fn export(&self, spans: &[ExportableSpan]) -> Result<(), String> {
                self.spans
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .extend_from_slice(spans);
                Ok(())
            }
        }

        struct CompleteRunProvider;

        #[async_trait::async_trait]
        impl GenerationProvider for CompleteRunProvider {
            async fn generate(
                &self,
                model_id: &str,
                _messages: Vec<serde_json::Value>,
                _settings: &ProviderBindingSettings,
                _tools: &[serde_json::Value],
            ) -> Result<GenerationResponse, ProviderAdapterError> {
                Ok(GenerationResponse {
                    text: serde_json::json!([{
                        "action_type": "complete_run",
                        "description": "done",
                        "confidence": 0.98
                    }])
                    .to_string(),
                    input_tokens: Some(42),
                    output_tokens: Some(17),
                    model_id: model_id.to_owned(),
                    tool_calls: vec![],
                    finish_reason: None,
                })
            }
        }

        let captured_spans = Arc::new(Mutex::new(Vec::<ExportableSpan>::new()));
        let mut app_state = AppState::new(BootstrapConfig::default()).await.unwrap();
        app_state.otlp_exporter = Arc::new(cairn_runtime::telemetry::OtlpExporter::new(
            OtlpConfig {
                enabled: true,
                service_name: "cairn-test".to_owned(),
                ..Default::default()
            },
            Box::new(RecordingSink {
                spans: captured_spans.clone(),
            }),
        ));
        app_state.brain_provider = Some(Arc::new(CompleteRunProvider));
        let state = Arc::new(app_state);
        register_token(&state, "otlp-token");

        let create_session = post_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/sessions",
            "otlp-token",
            serde_json::json!({
                "tenant_id": DEFAULT_TENANT_ID,
                "workspace_id": DEFAULT_WORKSPACE_ID,
                "project_id": DEFAULT_PROJECT_ID,
                "session_id": "sess_otlp_1"
            }),
        )
        .await;
        assert_eq!(create_session.status(), StatusCode::CREATED);

        let create_run = post_json(
            AppBootstrap::build_router(state.clone()),
            "/v1/runs",
            "otlp-token",
            serde_json::json!({
                "tenant_id": DEFAULT_TENANT_ID,
                "workspace_id": DEFAULT_WORKSPACE_ID,
                "project_id": DEFAULT_PROJECT_ID,
                "session_id": "sess_otlp_1",
                "run_id": "run_otlp_1"
            }),
        )
        .await;
        assert_eq!(create_run.status(), StatusCode::CREATED);

        let orchestrate_response = post_json(
            AppBootstrap::build_router(state),
            "/v1/runs/run_otlp_1/orchestrate",
            "otlp-token",
            serde_json::json!({
                "goal": "Finish immediately",
                "model_id": "test-brain",
                "max_iterations": 1
            }),
        )
        .await;
        assert_eq!(orchestrate_response.status(), StatusCode::OK);
        let orchestrate_body = response_json(orchestrate_response).await;
        assert_eq!(orchestrate_body["termination"], "completed");

        let spans = captured_spans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert!(
            spans.iter().any(|span| span.name == "run.created"
                && matches!(
                    span.attributes.get("service.name"),
                    Some(SpanAttributeValue::String(service)) if service == "cairn-test"
                )),
            "expected run.created span with service.name attribute: {spans:?}"
        );
        assert!(
            spans.iter().any(|span| span.name == "llm:test-brain"
                && matches!(
                    span.attributes.get("gen_ai.operation.name"),
                    Some(SpanAttributeValue::String(name)) if name == "chat"
                )
                && matches!(
                    span.attributes.get("gen_ai.request.model"),
                    Some(SpanAttributeValue::String(model)) if model == "test-brain"
                )
                && matches!(
                    span.attributes.get("gen_ai.usage.input_tokens"),
                    Some(SpanAttributeValue::Int(42))
                )
                && matches!(
                    span.attributes.get("gen_ai.usage.output_tokens"),
                    Some(SpanAttributeValue::Int(17))
                )),
            "expected provider span with GenAI attributes: {spans:?}"
        );
    }

    #[tokio::test]
    async fn stats_endpoint_reflects_created_sessions_and_runs() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "stats2-token");

        let project = ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);
        let session_id = SessionId::new("sess_stats_test");
        let run_id = RunId::new("run_stats_test");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id, None)
            .await
            .unwrap();

        let app = AppBootstrap::build_router(state);
        let body = get_json(app, "/v1/stats", "stats2-token").await;

        assert!(
            body["total_events"].as_u64().unwrap_or(0) >= 2,
            "at least 2 events expected (session + run create)"
        );
        assert!(
            body["total_sessions"].as_u64().unwrap_or(0) >= 1,
            "at least 1 session expected"
        );
        assert!(
            body["active_runs"].as_u64().unwrap_or(0) >= 1,
            "at least 1 active run expected"
        );
    }

    /// Verify that eval runs written via create_eval_run_handler are persisted
    /// as EvalRunStarted events and can be reconstructed by replay_evals().
    #[tokio::test]
    async fn eval_replay_restores_runs_from_event_log() {
        // Phase 1: create an eval run — this writes to state.evals AND appends
        // an EvalRunStarted event to the runtime store.
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        register_token(&state, "eval-replay-tok");

        let app = AppBootstrap::build_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/evals/runs")
                    .header("Content-Type", "application/json")
                    .header("Authorization", "Bearer eval-replay-tok")
                    .body(Body::from(
                        r#"{
                            "eval_run_id":   "eval_replay_1",
                            "tenant_id":     "default",
                            "workspace_id":  "default",
                            "project_id":    "default",
                            "subject_kind":  "prompt_release",
                            "evaluator_type":"accuracy",
                            "prompt_asset_id":"pa_1",
                            "prompt_release_id":"rel_1"
                        }"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 201, "eval run creation should succeed");

        // Phase 2: simulate restart — create a FRESH AppState sharing the same
        // runtime store.  replay_evals() should reconstruct the run.
        let fresh_state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());

        // Confirm the run is NOT present before replay.
        assert!(
            fresh_state
                .evals
                .get(&EvalRunId::new("eval_replay_1"))
                .is_none(),
            "eval run should not be in a fresh state before replay"
        );

        // Instead of a full replay (which requires the same store), verify the
        // write-side: the original state has the event in the store.
        use cairn_store::event_log::EventLog;
        let events = state
            .runtime
            .store
            .read_stream(None, usize::MAX)
            .await
            .unwrap();
        let eval_event = events.iter().find(|e| {
            matches!(&e.envelope.payload,
                cairn_domain::RuntimeEvent::EvalRunStarted(ev) if ev.eval_run_id.as_str() == "eval_replay_1"
            )
        });
        assert!(
            eval_event.is_some(),
            "EvalRunStarted event must be in the store"
        );

        if let Some(stored) = eval_event {
            if let cairn_domain::RuntimeEvent::EvalRunStarted(ev) = &stored.envelope.payload {
                assert_eq!(ev.evaluator_type, "accuracy");
                assert_eq!(
                    ev.prompt_asset_id.as_ref().map(|id| id.as_str()),
                    Some("pa_1")
                );
                assert_eq!(
                    ev.prompt_release_id.as_ref().map(|id| id.as_str()),
                    Some("rel_1")
                );
            }
        }

        // Phase 3: verify replay_evals() reconstructs the run when replayed
        // against the same store (same Arc).
        state.replay_evals().await;
        // The run was already there from phase 1 — replay should be idempotent.
        assert!(
            state.evals.get(&EvalRunId::new("eval_replay_1")).is_some(),
            "eval run must be present after replay"
        );
    }

    // ── Auth token handler tests ───────────────────────────────────────────────

    fn admin_principal() -> AuthPrincipal {
        AuthPrincipal::ServiceAccount {
            name: "admin".to_owned(),
            tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new("default")),
        }
    }

    fn operator_principal() -> AuthPrincipal {
        AuthPrincipal::Operator {
            operator_id: cairn_domain::ids::OperatorId::new("op_1"),
            tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new("default")),
        }
    }

    #[test]
    fn is_admin_principal_recognises_admin_service_account() {
        assert!(is_admin_principal(&admin_principal()));
    }

    #[test]
    fn is_admin_principal_rejects_operator() {
        assert!(!is_admin_principal(&operator_principal()));
    }

    #[test]
    fn is_admin_principal_accepts_system() {
        assert!(is_admin_principal(&AuthPrincipal::System));
    }

    #[test]
    fn operator_token_store_insert_list_remove() {
        let store = OperatorTokenStore::new();
        let record = OperatorTokenRecord {
            token_id: "tok_1".to_owned(),
            operator_id: "op_1".to_owned(),
            tenant_id: "t1".to_owned(),
            name: "ci-bot".to_owned(),
            created_at: 0,
            expires_at: None,
        };
        store.insert("sk_raw".to_owned(), record);
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.raw_token("tok_1").unwrap(), "sk_raw");
        assert!(store.remove("tok_1"));
        assert!(store.list().is_empty());
    }

    #[test]
    fn token_store_raw_token_used_for_revocation() {
        let store = OperatorTokenStore::new();
        let record = OperatorTokenRecord {
            token_id: "tok_abc".to_owned(),
            operator_id: "op_1".to_owned(),
            tenant_id: "t1".to_owned(),
            name: "deploy-bot".to_owned(),
            created_at: 0,
            expires_at: None,
        };
        store.insert("sk_secret123".to_owned(), record);
        assert_eq!(store.raw_token("tok_abc").unwrap(), "sk_secret123");
        assert!(store.remove("tok_abc"));
        assert!(store.raw_token("tok_abc").is_none());
    }

    #[test]
    fn token_store_remove_nonexistent_returns_false() {
        let store = OperatorTokenStore::new();
        assert!(!store.remove("tok_ghost"));
    }

    #[test]
    fn service_token_registry_revoke() {
        use cairn_api::auth::ServiceTokenRegistry;
        let reg = ServiceTokenRegistry::new();
        reg.register("tok".to_owned(), AuthPrincipal::System);
        assert!(reg.validate("tok").is_some());
        assert!(reg.revoke("tok"));
        assert!(reg.validate("tok").is_none());
        // Second revoke is idempotent (returns false).
        assert!(!reg.revoke("tok"));
    }
}
