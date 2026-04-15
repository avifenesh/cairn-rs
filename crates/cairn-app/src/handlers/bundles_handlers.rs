//! Bundle validate, plan, apply, and export handlers.
//!
//! Extracted from `lib.rs` — contains bundle validation, planning, application,
//! document export, filtered export, prompt export, and format-based export.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_domain::{ProjectKey, PromptAssetId};
use cairn_memory::bundles::{
    BundleEnvelope, ConflictResolutionStrategy, DocumentExportFilters, ImportService,
};

use crate::errors::{bad_request_response, AppApiError};
use crate::helpers::{parse_csv_values, parse_project_scope};
use crate::state::AppState;

const DEFAULT_TENANT_ID: &str = "default_tenant";

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct BundleExportQuery {
    pub project: Option<String>,
    pub tenant_id: Option<String>,
    pub workspace_id: Option<String>,
    pub project_id: Option<String>,
    pub source_ids: Option<String>,
    pub bundle_name: Option<String>,
}

impl BundleExportQuery {
    pub(crate) fn project(&self) -> Result<ProjectKey, &'static str> {
        if let Some(project) = self.project.as_deref() {
            if let Some((tenant_id, workspace_id, project_id)) = parse_project_scope(project) {
                return Ok(ProjectKey::new(tenant_id, workspace_id, project_id));
            }
            return Err("project must use tenant/workspace/project");
        }

        match (
            self.tenant_id.as_deref(),
            self.workspace_id.as_deref(),
            self.project_id.as_deref(),
        ) {
            (Some(tenant_id), Some(workspace_id), Some(project_id)) => {
                Ok(ProjectKey::new(tenant_id, workspace_id, project_id))
            }
            _ => Err("tenant_id, workspace_id, and project_id are required"),
        }
    }

    pub(crate) fn source_ids(&self) -> Vec<String> {
        self.source_ids
            .as_deref()
            .map(parse_csv_values)
            .unwrap_or_default()
    }

    pub(crate) fn bundle_name(&self) -> &str {
        self.bundle_name
            .as_deref()
            .unwrap_or("operator-document-export")
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct PromptBundleExportQuery {
    pub tenant_id: Option<String>,
    pub asset_ids: Option<String>,
    pub bundle_name: Option<String>,
}

impl PromptBundleExportQuery {
    pub(crate) fn tenant_id(&self) -> String {
        self.tenant_id
            .clone()
            .unwrap_or_else(|| DEFAULT_TENANT_ID.to_owned())
    }

    pub(crate) fn asset_ids(&self) -> Vec<PromptAssetId> {
        self.asset_ids
            .as_deref()
            .map(parse_csv_values)
            .unwrap_or_default()
            .into_iter()
            .map(PromptAssetId::new)
            .collect()
    }

    pub(crate) fn bundle_name(&self) -> &str {
        self.bundle_name
            .as_deref()
            .unwrap_or("operator-prompt-export")
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct ApplyBundleRequest {
    #[serde(default)]
    pub conflict_resolution: ConflictResolutionStrategy,
    #[serde(flatten)]
    pub bundle: BundleEnvelope,
}

/// Request body for POST /v1/bundles/export-filtered.
#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct ExportFilteredRequest {
    pub project: Option<String>,
    pub tenant_id: Option<String>,
    pub workspace_id: Option<String>,
    pub project_id: Option<String>,
    pub bundle_name: Option<String>,
    #[serde(default)]
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_after_ms: Option<u64>,
    pub created_before_ms: Option<u64>,
    pub min_credibility_score: Option<f32>,
}

impl ExportFilteredRequest {
    pub(crate) fn project(&self) -> Result<ProjectKey, &'static str> {
        if let Some(project) = self.project.as_deref() {
            if let Some((tenant_id, workspace_id, project_id)) = parse_project_scope(project) {
                return Ok(ProjectKey::new(tenant_id, workspace_id, project_id));
            }
            return Err("project must use tenant/workspace/project");
        }
        match (
            self.tenant_id.as_deref(),
            self.workspace_id.as_deref(),
            self.project_id.as_deref(),
        ) {
            (Some(tenant_id), Some(workspace_id), Some(project_id)) => {
                Ok(ProjectKey::new(tenant_id, workspace_id, project_id))
            }
            _ => Err("tenant_id, workspace_id, and project_id are required"),
        }
    }

    pub(crate) fn bundle_name(&self) -> &str {
        self.bundle_name
            .as_deref()
            .unwrap_or("operator-document-export")
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

pub(crate) async fn validate_bundle_handler(
    State(state): State<Arc<AppState>>,
    Json(bundle): Json<BundleEnvelope>,
) -> impl IntoResponse {
    match state.bundle_import.validate(&bundle).await {
        Ok(report) if report.valid => (StatusCode::OK, Json(report)).into_response(),
        Ok(report) => (StatusCode::UNPROCESSABLE_ENTITY, Json(report)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err).into_response(),
    }
}

pub(crate) async fn plan_bundle_handler(
    State(state): State<Arc<AppState>>,
    Json(bundle): Json<BundleEnvelope>,
) -> impl IntoResponse {
    let validation = match state.bundle_import.validate(&bundle).await {
        Ok(report) => report,
        Err(err) => {
            return AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err).into_response()
        }
    };
    if !validation.valid {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(validation)).into_response();
    }

    match state
        .bundle_import
        .plan(&bundle, &bundle.source_scope)
        .await
    {
        Ok(plan) => (StatusCode::OK, Json(plan)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err).into_response(),
    }
}

pub(crate) async fn apply_bundle_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ApplyBundleRequest>,
) -> impl IntoResponse {
    let bundle = request.bundle;
    let validation = match state.bundle_import.validate(&bundle).await {
        Ok(report) => report,
        Err(err) => {
            return AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err).into_response()
        }
    };
    if !validation.valid {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(validation)).into_response();
    }

    let plan = match state
        .bundle_import
        .plan(&bundle, &bundle.source_scope)
        .await
    {
        Ok(mut plan) => {
            plan.conflict_resolution = request.conflict_resolution;
            plan
        }
        Err(err) => {
            return AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err).into_response()
        }
    };

    match state.bundle_import.apply(&plan, &bundle).await {
        Ok(report) => (StatusCode::OK, Json(report)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err).into_response(),
    }
}

pub(crate) async fn export_bundle_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BundleExportQuery>,
) -> impl IntoResponse {
    let project = match query.project() {
        Ok(project) => project,
        Err(err) => return bad_request_response(err),
    };
    let filters = DocumentExportFilters {
        bundle_source_id: None,
        import_id: None,
        source_ids: query.source_ids(),
        tags: vec![],
        created_after_ms: None,
        created_before_ms: None,
        min_credibility_score: None,
        corpus_id: None,
        created_at: None,
        min_quality_score: None,
    };

    match state
        .bundle_export
        .export_documents(query.bundle_name(), &project, &filters)
        .await
    {
        Ok(bundle) => (StatusCode::OK, Json(bundle)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err).into_response(),
    }
}

pub(crate) async fn export_filtered_bundle_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ExportFilteredRequest>,
) -> impl IntoResponse {
    let project = match body.project() {
        Ok(project) => project,
        Err(err) => return bad_request_response(err),
    };
    let bundle_name = body.bundle_name().to_owned();
    let filters = DocumentExportFilters {
        bundle_source_id: None,
        import_id: None,
        source_ids: body.source_ids,
        tags: body.tags,
        created_after_ms: body.created_after_ms,
        created_before_ms: body.created_before_ms,
        min_credibility_score: body.min_credibility_score,
        corpus_id: None,
        created_at: None,
        min_quality_score: None,
    };
    match state
        .bundle_export
        .export_documents(&bundle_name, &project, &filters)
        .await
    {
        Ok(bundle) => (StatusCode::OK, Json(bundle)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err).into_response(),
    }
}

pub(crate) async fn export_prompt_bundle_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PromptBundleExportQuery>,
) -> impl IntoResponse {
    match state
        .bundle_export
        .export_prompts(query.bundle_name(), &query.tenant_id(), &query.asset_ids())
        .await
    {
        Ok(bundle) => (StatusCode::OK, Json(bundle)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err).into_response(),
    }
}

pub(crate) async fn export_bundle_by_format_handler(
    Path(format): Path<String>,
) -> impl IntoResponse {
    AppApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "not_implemented",
        format!(
            "Export format '{}' is not yet implemented. Planned: json, yaml, csv.",
            format
        ),
    )
}
