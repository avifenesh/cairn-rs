//! HTTP router construction: merges catalog + binary routes,
//! applies middleware, and falls back to the embedded frontend.

#[allow(unused_imports)]
use crate::*;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use tower_http::trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::Level;

pub(crate) fn build_router(lib_state: Arc<cairn_app::AppState>, state: AppState) -> Router {
    // ── Base: 197 catalog-driven routes from lib.rs ──────────────────────
    let catalog_routes =
        cairn_app::AppBootstrap::build_catalog_routes().with_state(lib_state.clone());

    // ── Binary-specific routes (not in the catalog) ──────────────────────
    let binary_routes: Router = Router::new()
        // WebSocket (catalog handles /v1/stream and /v1/streams/runtime)
        .route("/v1/ws", get(ws_handler))
        // System introspection
        .route("/v1/health/detailed", get(detailed_health_handler))
        .route("/v1/system/info", get(system_info_handler))
        .route("/v1/system/role", get(system_role_handler))
        // /v1/overview served by catalog
        // Runs — binary-specific views
        .route("/v1/runs/batch", post(batch_create_runs_handler))
        .route(
            "/v1/runs/:id/tool-invocations",
            get(list_run_tool_invocations_handler),
        )
        .route(
            "/v1/runs/:id/tasks",
            get(list_run_tasks_handler).post(create_run_task_handler),
        )
        .route("/v1/runs/:id/approvals", get(list_run_approvals_handler))
        .route("/v1/runs/:id/export", get(export_run_handler))
        // Sessions — binary-specific views
        .route("/v1/sessions/import", post(import_session_handler))
        .route("/v1/sessions/:id/runs", get(list_session_runs_handler))
        .route("/v1/sessions/:id/export", get(export_session_handler))
        // Approvals — /resolve as primary per W3 decision
        .route("/v1/approvals/pending", get(list_pending_approvals_handler))
        .route("/v1/approvals/:id/resolve", post(resolve_approval_handler))
        // Events
        .route("/v1/events", get(list_events_handler))
        .route("/v1/events/append", post(append_events_handler))
        // Tasks — binary-specific (complete served by catalog)
        .route("/v1/tasks/batch/cancel", post(batch_cancel_tasks_handler))
        .route("/v1/tasks/:id/start", post(start_task_handler))
        .route("/v1/tasks/:id/fail", post(fail_task_handler))
        // Traces
        .route("/v1/traces", get(list_all_traces_handler))
        .route("/v1/traces/export", get(export_otlp_handler))
        // Admin utilities
        // NOTE: /v1/admin/logs is now served by the catalog-driven router in lib.rs.
        // The observability middleware populates lib_state.request_log, which the
        // catalog handler reads — so the request log is always fresh.
        .route("/v1/admin/snapshot", post(admin_snapshot_handler))
        .route("/v1/admin/backup", post(backup_handler))
        .route("/v1/admin/restore", post(admin_restore_handler))
        .route(
            "/v1/admin/rebuild-projections",
            post(rebuild_projections_handler),
        )
        .route("/v1/admin/event-count", get(event_count_handler))
        .route("/v1/admin/event-log", get(admin_event_log_handler))
        .route("/v1/admin/rotate-token", post(rotate_token_handler))
        // Notifications
        .route("/v1/notifications", get(list_notifications_handler))
        .route(
            "/v1/notifications/read-all",
            post(mark_all_notifications_read_handler),
        )
        .route(
            "/v1/notifications/:id/read",
            post(mark_notification_read_handler),
        )
        // Bundles — /import aliases to /apply per W3 decision
        .route("/v1/bundles/import", post(bundle_import_handler))
        // Entitlements
        .route("/v1/entitlements", get(entitlements_handler))
        .route("/v1/entitlements/usage", get(entitlements_usage_handler))
        // Templates
        .route("/v1/templates", get(list_templates_handler))
        .route("/v1/templates/:id", get(get_template_handler))
        .route("/v1/templates/:id/apply", post(apply_template_handler))
        // Ollama local LLM provider
        // Provider connection discovery + health test
        .route(
            "/v1/providers/connections/:id/discover-models",
            get(discover_models_handler),
        )
        .route(
            "/v1/providers/connections/:id/test",
            get(test_connection_handler),
        )
        .route("/v1/providers/ollama/models", get(ollama_models_handler))
        .route(
            "/v1/providers/ollama/models/:name/info",
            get(ollama_model_info_handler),
        )
        .route(
            "/v1/providers/ollama/generate",
            post(ollama_generate_handler),
        )
        .route("/v1/chat/stream", post(chat_stream_handler))
        // Keep the old route as an alias for backwards compatibility
        .route("/v1/providers/ollama/stream", post(chat_stream_handler))
        .route("/v1/providers/ollama/pull", post(ollama_pull_handler))
        .route(
            "/v1/providers/ollama/delete",
            post(ollama_delete_model_handler),
        )
        .route("/v1/memory/embed", post(ollama_embed_handler))
        // Database diagnostics
        .route("/v1/db/status", get(db_status_handler))
        // /v1/metrics served by catalog; prometheus is binary-only
        .route("/v1/metrics/prometheus", get(metrics_prometheus_handler))
        // OpenAPI + docs
        .route("/v1/openapi.json", get(openapi_json_handler))
        .route("/v1/docs", get(swagger_ui_handler))
        .route("/v1/changelog", get(changelog_handler))
        // Test
        .route("/v1/test/webhook", post(test_webhook_handler))
        // Rate-limit status
        .route("/v1/rate-limit", get(rate_limit_status_handler))
        .with_state(state);

    // ── Merge catalog + binary routes ────────────────────────────────────
    let merged = catalog_routes
        .merge(binary_routes)
        .fallback(get(serve_frontend));

    // ── Apply lib.rs middleware (auth, CORS, rate-limit, tracing) ────────
    cairn_app::AppBootstrap::apply_middleware(merged, lib_state)
        // Binary-specific outer layers
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(axum::middleware::from_fn(version_header_middleware))
}
