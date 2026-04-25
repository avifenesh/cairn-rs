//! F29 CD-2: project / workspace cost rollup HTTP handlers.
//!
//! Both endpoints read from the same `ProjectCostReadModel` that the
//! `SessionCostUpdated` projection writes to, so the numbers returned
//! here are the sum of the per-session breakdown exposed by
//! `GET /v1/sessions/:id/cost`. Lifetime-total in v1; time-range slicing
//! is a follow-up.
//!
//! Scope enforcement: the tenant in the URL MUST match the caller's
//! authenticated tenant unless the caller is an admin service account.
//! The check is explicit here (not middleware) so a future admin-only
//! path that wants to see any tenant's totals stays easy to add.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_domain::ProjectKey;
use cairn_store::projections::ProjectCostReadModel;

use crate::errors::{store_error_response, AppApiError};
use crate::extractors::TenantScope;
use crate::state::AppState;

/// Response payload for `GET /v1/projects/:tenant/:workspace/:project/costs`.
///
/// Shape is `#[serde(flatten)]` of the record so the JSON is a flat
/// object — matches the `SessionCostResponse` convention and keeps UI
/// parsing trivial.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct ProjectCostSummary {
    #[serde(flatten)]
    pub(crate) record: cairn_domain::providers::ProjectCostRecord,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct WorkspaceCostSummary {
    #[serde(flatten)]
    pub(crate) record: cairn_domain::providers::WorkspaceCostRecord,
}

/// Reject a cross-tenant read. Mirrors the `ProjectJson` middleware
/// check: admins bypass, everyone else must match.
fn enforce_tenant(scope: &TenantScope, url_tenant: &str) -> Option<AppApiError> {
    if scope.is_admin {
        return None;
    }
    if scope.tenant_id().as_str() == url_tenant {
        return None;
    }
    Some(AppApiError::new(
        StatusCode::FORBIDDEN,
        "forbidden",
        "tenant scope mismatch",
    ))
}

/// `GET /v1/projects/:tenant/:workspace/:project/costs`
pub(crate) async fn get_project_costs_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path((tenant_id, workspace_id, project_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if let Some(err) = enforce_tenant(&tenant_scope, &tenant_id) {
        return err.into_response();
    }
    let project = ProjectKey::new(
        tenant_id.as_str(),
        workspace_id.as_str(),
        project_id.as_str(),
    );
    match ProjectCostReadModel::get_project_cost(state.runtime.store.as_ref(), &project).await {
        Ok(Some(record)) => (StatusCode::OK, Json(ProjectCostSummary { record })).into_response(),
        // Empty rollup is not an error — zero is a valid answer for a
        // project that hasn't emitted any provider calls yet. Returning
        // 200 with zeros lets the UI render the panel without special-
        // casing "not-found".
        Ok(None) => {
            let zero = cairn_domain::providers::ProjectCostRecord {
                tenant_id: cairn_domain::TenantId::new(&tenant_id),
                workspace_id: workspace_id.clone(),
                project_id: project_id.clone(),
                total_cost_micros: 0,
                total_tokens_in: 0,
                total_tokens_out: 0,
                provider_calls: 0,
                updated_at_ms: 0,
            };
            (StatusCode::OK, Json(ProjectCostSummary { record: zero })).into_response()
        }
        Err(err) => store_error_response(err),
    }
}

/// `GET /v1/workspaces/:tenant/:workspace/costs`
pub(crate) async fn get_workspace_costs_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path((tenant_id, workspace_id)): Path<(String, String)>,
) -> impl IntoResponse {
    if let Some(err) = enforce_tenant(&tenant_scope, &tenant_id) {
        return err.into_response();
    }
    let tid = cairn_domain::TenantId::new(&tenant_id);
    match ProjectCostReadModel::get_workspace_cost(
        state.runtime.store.as_ref(),
        &tid,
        &workspace_id,
    )
    .await
    {
        Ok(Some(record)) => (StatusCode::OK, Json(WorkspaceCostSummary { record })).into_response(),
        Ok(None) => {
            let zero = cairn_domain::providers::WorkspaceCostRecord {
                tenant_id: tid,
                workspace_id: workspace_id.clone(),
                total_cost_micros: 0,
                total_tokens_in: 0,
                total_tokens_out: 0,
                provider_calls: 0,
                updated_at_ms: 0,
            };
            (StatusCode::OK, Json(WorkspaceCostSummary { record: zero })).into_response()
        }
        Err(err) => store_error_response(err),
    }
}
