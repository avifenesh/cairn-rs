//! HTTP route handlers for the plugin marketplace — RFC 015.
//!
//! Routes:
//!   GET    /v1/plugins/catalog          → list_catalog_handler
//!   POST   /v1/plugins/:id/install      → install_plugin_handler
//!   POST   /v1/plugins/:id/credentials  → provide_credentials_handler
//!   POST   /v1/plugins/:id/verify       → verify_credentials_handler
//!   POST   /v1/projects/:proj/plugins/:id  → enable_plugin_handler
//!   DELETE /v1/projects/:proj/plugins/:id  → disable_plugin_handler
//!   DELETE /v1/plugins/:id              → uninstall_plugin_handler

use std::sync::Arc;

use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use cairn_api::auth::AuthPrincipal;
use cairn_domain::contexts::SignalCaptureOverride;
use cairn_runtime::{
    CredentialScopeKey, MarketplaceCommand, MarketplaceEvent, MarketplaceRecord, MarketplaceState,
};

use crate::AppState;

// ── Response DTOs ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct CatalogEntryResponse {
    id: String,
    name: String,
    version: String,
    description: Option<String>,
    category: String,
    vendor: String,
    state: String,
    tools_count: usize,
    signals_count: usize,
    download_url: Option<String>,
    has_signal_source: bool,
}

#[derive(Serialize)]
struct CatalogResponse {
    plugins: Vec<CatalogEntryResponse>,
}

#[derive(Serialize)]
struct PluginEventResponse {
    events: Vec<MarketplaceEvent>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

// ── Request DTOs ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ProvideCredentialsRequest {
    pub credentials: Vec<CredentialEntry>,
}

#[derive(Deserialize)]
pub struct CredentialEntry {
    pub key: String,
    pub value: String,
}

#[derive(Deserialize)]
pub struct VerifyCredentialsRequest {
    pub credential_scope_key: Option<String>,
}

#[derive(Deserialize)]
pub struct EnablePluginRequest {
    pub tool_allowlist: Option<Vec<String>>,
    pub signal_allowlist: Option<Vec<String>>,
    pub signal_capture_override: Option<SignalCaptureOverride>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ListQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl ListQuery {
    fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(100)
    }

    fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn record_to_response(record: &MarketplaceRecord) -> CatalogEntryResponse {
    let state = match &record.state {
        MarketplaceState::Listed => "listed",
        MarketplaceState::Installing => "installing",
        MarketplaceState::Installed => "installed",
        MarketplaceState::InstallationFailed { .. } => "installation_failed",
        MarketplaceState::Uninstalled => "uninstalled",
    };

    CatalogEntryResponse {
        id: record.descriptor.id.clone(),
        name: record.descriptor.name.clone(),
        version: record.descriptor.version.clone(),
        description: record.descriptor.description.clone(),
        category: format!("{:?}", record.descriptor.category),
        vendor: record.descriptor.vendor.clone(),
        state: state.to_string(),
        tools_count: record.descriptor.tools.len(),
        signals_count: record.descriptor.signal_sources.len(),
        download_url: record.descriptor.download_url.clone(),
        has_signal_source: record.descriptor.has_signal_source,
    }
}

fn operator_id_from_principal(
    principal: &cairn_api::auth::AuthPrincipal,
) -> cairn_domain::ids::OperatorId {
    // T6b-C5: derive the operator id from the authenticated principal
    // so plugin install / credential writes land with the real actor
    // in the event log.
    cairn_domain::ids::OperatorId::new(crate::handlers::admin::audit_actor_id(principal))
}

use crate::extractors::enforce_project_tenant;

fn validate_project_segment(value: &str, field: &'static str) -> Result<(), String> {
    let is_valid = !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));

    if is_valid {
        Ok(())
    } else {
        Err(format!("{field} contains unsupported path characters"))
    }
}

fn project_key_from_path(project: &str) -> Result<cairn_domain::tenancy::ProjectKey, String> {
    if let Some((tenant_id, workspace_id, project_id)) = crate::parse_project_scope(project) {
        validate_project_segment(tenant_id, "tenant_id")?;
        validate_project_segment(workspace_id, "workspace_id")?;
        validate_project_segment(project_id, "project_id")?;
        return Ok(cairn_domain::tenancy::ProjectKey::new(
            tenant_id,
            workspace_id,
            project_id,
        ));
    }

    validate_project_segment(project, "project_id")?;
    Ok(cairn_domain::tenancy::ProjectKey::new(
        crate::DEFAULT_TENANT_ID,
        crate::DEFAULT_WORKSPACE_ID,
        project,
    ))
}

fn bad_request_response(message: impl Into<String>) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// GET /v1/plugins/catalog
///
/// Lists all known plugin descriptors with their marketplace state.
pub async fn list_catalog_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> impl IntoResponse {
    let marketplace = state.marketplace.lock().unwrap_or_else(|e| e.into_inner());
    let mut records = marketplace.list_all_records();
    records.sort_by_key(|r| r.descriptor.id.clone());
    let plugins = records
        .into_iter()
        .skip(query.offset())
        .take(query.limit())
        .map(record_to_response)
        .collect();
    (StatusCode::OK, Json(CatalogResponse { plugins })).into_response()
}

/// POST /v1/plugins/:id/install
///
/// Installs a listed plugin, transitioning it from Listed to Installed.
pub async fn install_plugin_handler(
    State(state): State<Arc<AppState>>,
    _role: crate::extractors::AdminRoleGuard,
    Extension(principal): Extension<AuthPrincipal>,
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    // T6b-C5: plugin install is tenant-wide and admin-only.
    let operator = operator_id_from_principal(&principal);
    let mut marketplace = state.marketplace.lock().unwrap_or_else(|e| e.into_inner());

    match marketplace.handle_command(MarketplaceCommand::InstallPlugin {
        plugin_id,
        initiated_by: operator,
    }) {
        Ok(events) => (StatusCode::OK, Json(PluginEventResponse { events })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// POST /v1/plugins/:id/credentials
///
/// Provides credentials for an installed plugin via the credential wizard.
pub async fn provide_credentials_handler(
    State(state): State<Arc<AppState>>,
    _role: crate::extractors::AdminRoleGuard,
    Extension(principal): Extension<AuthPrincipal>,
    Path(plugin_id): Path<String>,
    Json(body): Json<ProvideCredentialsRequest>,
) -> impl IntoResponse {
    // T6b-C5: plugin credential writes touch the shared credential
    // store — admin-gated.
    let operator = operator_id_from_principal(&principal);
    let mut marketplace = state.marketplace.lock().unwrap_or_else(|e| e.into_inner());

    let credentials: Vec<(String, String)> = body
        .credentials
        .into_iter()
        .map(|c| (c.key, c.value))
        .collect();

    match marketplace.handle_command(MarketplaceCommand::ProvidePluginCredentials {
        plugin_id,
        credentials,
        provided_by: operator,
    }) {
        Ok(events) => (StatusCode::OK, Json(PluginEventResponse { events })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// POST /v1/plugins/:id/verify
///
/// Ephemeral credential verification — spawns a transient process,
/// runs the declared health check, and shuts down.
/// Does NOT commit any persistent lifecycle state.
pub async fn verify_credentials_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(plugin_id): Path<String>,
    body: Option<Json<VerifyCredentialsRequest>>,
) -> impl IntoResponse {
    let operator = operator_id_from_principal(&principal);
    let mut marketplace = state.marketplace.lock().unwrap_or_else(|e| e.into_inner());

    let scope_key = body.and_then(|Json(b)| b.credential_scope_key.map(CredentialScopeKey));

    match marketplace.handle_command(MarketplaceCommand::VerifyPluginCredentials {
        plugin_id,
        credential_scope_key: scope_key,
        verified_by: operator,
    }) {
        Ok(events) => (StatusCode::OK, Json(PluginEventResponse { events })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// POST /v1/projects/:proj/plugins/:id
///
/// Enables a plugin for a project with optional tool/signal allowlists
/// and signal capture override.
pub async fn enable_plugin_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path((project_id, plugin_id)): Path<(String, String)>,
    body: Option<Json<EnablePluginRequest>>,
) -> impl IntoResponse {
    let operator = operator_id_from_principal(&principal);

    let project = match project_key_from_path(&project_id) {
        Ok(project) => project,
        Err(message) => return bad_request_response(message),
    };

    // T6b-C5: refuse cross-tenant enable. Only the authenticated
    // tenant's projects may have plugins enabled. Admin bypass allowed.
    if !enforce_project_tenant(&principal, &project) {
        return crate::errors::tenant_scope_mismatch_error().into_response();
    }

    let mut marketplace = state.marketplace.lock().unwrap_or_else(|e| e.into_inner());

    let (tool_allowlist, signal_allowlist, signal_capture_override) = match body {
        Some(Json(b)) => (
            b.tool_allowlist,
            b.signal_allowlist,
            b.signal_capture_override,
        ),
        None => (None, None, None),
    };

    // Check if the plugin has SignalSource before enabling (for eager spawn)
    let has_signal_source = marketplace
        .get_record(&plugin_id)
        .map(|r| r.descriptor.has_signal_source)
        .unwrap_or(false);

    match marketplace.handle_command(MarketplaceCommand::EnablePluginForProject {
        plugin_id: plugin_id.clone(),
        project,
        tool_allowlist,
        signal_allowlist,
        signal_capture_override,
        enabled_by: operator,
    }) {
        Ok(events) => {
            // RFC 015 spawn policy: SignalSource-declaring plugins eager-spawn
            // at EnablePluginForProject so webhook ingress has a listener.
            if has_signal_source {
                if let Some(record) = marketplace.get_record(&plugin_id) {
                    let manifest = cairn_tools::PluginManifest {
                        id: record.descriptor.id.clone(),
                        name: record.descriptor.name.clone(),
                        version: record.descriptor.version.clone(),
                        command: record.descriptor.command.clone(),
                        capabilities: Vec::new(),
                        permissions: cairn_tools::DeclaredPermissions::default(),
                        limits: None,
                        execution_class: cairn_domain::policy::ExecutionClass::SandboxedProcess,
                        description: record.descriptor.description.clone(),
                        homepage: None,
                    };
                    drop(marketplace); // release Mutex before locking plugin_host
                    if let Ok(mut host) = state.plugin_host.lock() {
                        // Register with host — process spawns on first use or handshake
                        let _ = host.register(manifest);
                    }
                }
            }
            (StatusCode::OK, Json(PluginEventResponse { events })).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// DELETE /v1/projects/:proj/plugins/:id
///
/// Disables a plugin for a project.
pub async fn disable_plugin_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path((project_id, plugin_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let operator = operator_id_from_principal(&principal);

    let project = match project_key_from_path(&project_id) {
        Ok(project) => project,
        Err(message) => return bad_request_response(message),
    };

    // T6b-C5: refuse cross-tenant disable.
    if !enforce_project_tenant(&principal, &project) {
        return crate::errors::tenant_scope_mismatch_error().into_response();
    }

    let mut marketplace = state.marketplace.lock().unwrap_or_else(|e| e.into_inner());

    match marketplace.handle_command(MarketplaceCommand::DisablePluginForProject {
        plugin_id,
        project,
        disabled_by: operator,
    }) {
        Ok(events) => (StatusCode::OK, Json(PluginEventResponse { events })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// DELETE /v1/plugins/:id
///
/// Uninstalls a plugin, revoking all credentials and removing all
/// project enablements.
pub async fn uninstall_plugin_handler(
    State(state): State<Arc<AppState>>,
    _role: crate::extractors::AdminRoleGuard,
    Extension(principal): Extension<AuthPrincipal>,
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    // T6b-C5: uninstall is tenant-wide and admin-only.
    let operator = operator_id_from_principal(&principal);
    let mut marketplace = state.marketplace.lock().unwrap_or_else(|e| e.into_inner());

    match marketplace.handle_command(MarketplaceCommand::UninstallPlugin {
        plugin_id,
        uninstalled_by: operator,
    }) {
        Ok(events) => (StatusCode::OK, Json(PluginEventResponse { events })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::{project_key_from_path, ListQuery};

    #[test]
    fn project_key_from_path_accepts_full_scope() {
        let scoped = project_key_from_path("tenant-a/workspace-a/project-a").unwrap();
        assert_eq!(scoped.tenant_id.as_str(), "tenant-a");
        assert_eq!(scoped.workspace_id.as_str(), "workspace-a");
        assert_eq!(scoped.project_id.as_str(), "project-a");
    }

    #[test]
    fn list_query_caps_page_size() {
        let query = ListQuery {
            limit: Some(500),
            offset: Some(3),
        };

        assert_eq!(query.limit(), 100);
        assert_eq!(query.offset(), 3);
    }
}
