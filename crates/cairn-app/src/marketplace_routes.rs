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

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use cairn_domain::contexts::SignalCaptureOverride;
use cairn_runtime::services::marketplace_service::{
    MarketplaceCommand, MarketplaceEvent, MarketplaceRecord, MarketplaceState,
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

fn operator_id_from_state(_state: &AppState) -> cairn_domain::ids::OperatorId {
    // TODO: extract from auth context once wired
    cairn_domain::ids::OperatorId::new("operator")
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// GET /v1/plugins/catalog
///
/// Lists all known plugin descriptors with their marketplace state.
pub async fn list_catalog_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let marketplace = state.marketplace.lock().unwrap();
    let records = marketplace.list_all_records();
    let plugins: Vec<CatalogEntryResponse> =
        records.iter().map(|r| record_to_response(r)).collect();
    Json(CatalogResponse { plugins })
}

/// POST /v1/plugins/:id/install
///
/// Installs a listed plugin, transitioning it from Listed to Installed.
pub async fn install_plugin_handler(
    State(state): State<Arc<AppState>>,
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    let operator = operator_id_from_state(&state);
    let mut marketplace = state.marketplace.lock().unwrap();

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
    Path(plugin_id): Path<String>,
    Json(body): Json<ProvideCredentialsRequest>,
) -> impl IntoResponse {
    let operator = operator_id_from_state(&state);
    let mut marketplace = state.marketplace.lock().unwrap();

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
    Path(plugin_id): Path<String>,
    body: Option<Json<VerifyCredentialsRequest>>,
) -> impl IntoResponse {
    let operator = operator_id_from_state(&state);
    let mut marketplace = state.marketplace.lock().unwrap();

    let scope_key = body.and_then(|Json(b)| {
        b.credential_scope_key
            .map(cairn_runtime::services::marketplace_service::CredentialScopeKey)
    });

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
    Path((project_id, plugin_id)): Path<(String, String)>,
    body: Option<Json<EnablePluginRequest>>,
) -> impl IntoResponse {
    let operator = operator_id_from_state(&state);
    let mut marketplace = state.marketplace.lock().unwrap();

    // TODO: resolve full ProjectKey from project_id via project service
    let project = cairn_domain::tenancy::ProjectKey::new("default", "default", project_id.as_str());

    let (tool_allowlist, signal_allowlist, signal_capture_override) = match body {
        Some(Json(b)) => (
            b.tool_allowlist,
            b.signal_allowlist,
            b.signal_capture_override,
        ),
        None => (None, None, None),
    };

    match marketplace.handle_command(MarketplaceCommand::EnablePluginForProject {
        plugin_id,
        project,
        tool_allowlist,
        signal_allowlist,
        signal_capture_override,
        enabled_by: operator,
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

/// DELETE /v1/projects/:proj/plugins/:id
///
/// Disables a plugin for a project.
pub async fn disable_plugin_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, plugin_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let operator = operator_id_from_state(&state);
    let mut marketplace = state.marketplace.lock().unwrap();

    let project = cairn_domain::tenancy::ProjectKey::new("default", "default", project_id.as_str());

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
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    let operator = operator_id_from_state(&state);
    let mut marketplace = state.marketplace.lock().unwrap();

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
