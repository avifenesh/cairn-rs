//! Approval policy and request handlers.
//!
//! Extracted from `lib.rs` — contains list/create approval policies,
//! request/approve/reject/deny/delegate approval endpoints.

use std::sync::Arc;

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::auth::AuthPrincipal;
use cairn_api::http::ListResponse;
use cairn_domain::{
    ApprovalDecision, ApprovalId, ApprovalRequirement, AuditOutcome, RunId, TaskId, TenantId,
    WorkspaceRole,
};
use cairn_runtime::{ApprovalPolicyService, ApprovalService, AuditService};

use crate::errors::{runtime_error_response, AppApiError};
use crate::extractors::{OptionalProjectScopedQuery, TenantScope};
use crate::handlers::admin::audit_actor_id;
use crate::state::AppState;

/// T6a-C1 helper: load approval and verify it belongs to the caller's tenant.
/// Returns 404 on missing; 404 (not 403) on cross-tenant mismatch so the
/// handler doesn't leak approval existence across tenants.
async fn load_approval_visible_to_tenant(
    state: &AppState,
    tenant_scope: &TenantScope,
    approval_id: &ApprovalId,
) -> Result<cairn_store::projections::ApprovalRecord, axum::response::Response> {
    match state.runtime.approvals.get(approval_id).await {
        Ok(Some(record))
            if tenant_scope.is_admin || record.project.tenant_id == *tenant_scope.tenant_id() =>
        {
            Ok(record)
        }
        Ok(_) => Err(
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "approval not found")
                .into_response(),
        ),
        Err(err) => Err(runtime_error_response(err)),
    }
}

const DEFAULT_TENANT_ID: &str = "default_tenant";

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateApprovalPolicyRequest {
    pub tenant_id: Option<String>,
    pub name: String,
    pub required_approvers: u32,
    pub allowed_approver_roles: Vec<WorkspaceRole>,
    pub auto_approve_after_ms: Option<u64>,
    pub auto_reject_after_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct ApprovalPolicyListQuery {
    pub tenant_id: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl ApprovalPolicyListQuery {
    pub(crate) fn tenant_id(&self) -> TenantId {
        TenantId::new(self.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID))
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct RequestApprovalRequest {
    pub tenant_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub approval_id: String,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub requirement: Option<ApprovalRequirement>,
    pub policy_id: Option<String>,
}

impl RequestApprovalRequest {
    pub(crate) fn project(&self) -> cairn_domain::ProjectKey {
        cairn_domain::ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct DelegateApprovalRequest {
    pub delegated_to: String,
}

// ── Handlers ────────────────────────────────────────────────────────────────

pub(crate) async fn list_approvals_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .approvals
        .list_all(&query.project(), query.limit(), query.offset())
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn create_approval_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateApprovalPolicyRequest>,
) -> impl IntoResponse {
    let tenant_id = TenantId::new(body.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID));
    match state
        .runtime
        .approval_policies
        .create(
            tenant_id,
            body.name,
            body.required_approvers,
            body.allowed_approver_roles,
            body.auto_approve_after_ms,
            body.auto_reject_after_ms,
        )
        .await
    {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_approval_policies_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ApprovalPolicyListQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .approval_policies
        .list(&query.tenant_id(), query.limit(), query.offset())
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse {
                has_more: items.len() == query.limit(),
                items,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn request_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Json(body): Json<RequestApprovalRequest>,
) -> impl IntoResponse {
    let project = body.project();
    // T6a-C1: the body-supplied project MUST match the authenticated tenant.
    // Admin bypass is allowed.
    if !tenant_scope.is_admin && project.tenant_id != *tenant_scope.tenant_id() {
        return crate::errors::tenant_scope_mismatch_error().into_response();
    }
    let before = crate::handlers::sse::current_event_head(&state).await;
    match state
        .runtime
        .approvals
        .request(
            &project,
            ApprovalId::new(body.approval_id),
            body.run_id.map(RunId::new),
            body.task_id.map(TaskId::new),
            body.requirement.unwrap_or(ApprovalRequirement::Required),
        )
        .await
    {
        Ok(record) => {
            crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
            (StatusCode::CREATED, Json(record)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn approve_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let approval_id = ApprovalId::new(id);
    // T6a-C1: verify tenant scope before any mutation.
    if let Err(response) =
        load_approval_visible_to_tenant(state.as_ref(), &tenant_scope, &approval_id).await
    {
        return response;
    }
    let before = crate::handlers::sse::current_event_head(&state).await;
    match state
        .runtime
        .approvals
        .resolve(&approval_id, ApprovalDecision::Approved)
        .await
    {
        Ok(record) => match state
            .runtime
            .audits
            .record(
                record.project.tenant_id.clone(),
                audit_actor_id(&principal),
                "resolve_approval".to_owned(),
                "approval".to_owned(),
                record.approval_id.to_string(),
                AuditOutcome::Success,
                serde_json::json!({ "decision": "approved" }),
            )
            .await
        {
            Ok(_) => {
                crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
                (StatusCode::OK, Json(record)).into_response()
            }
            Err(err) => runtime_error_response(err),
        },
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn reject_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let approval_id = ApprovalId::new(id);
    if let Err(response) =
        load_approval_visible_to_tenant(state.as_ref(), &tenant_scope, &approval_id).await
    {
        return response;
    }
    let before = crate::handlers::sse::current_event_head(&state).await;
    match state
        .runtime
        .approvals
        .resolve(&approval_id, ApprovalDecision::Rejected)
        .await
    {
        Ok(record) => match state
            .runtime
            .audits
            .record(
                record.project.tenant_id.clone(),
                audit_actor_id(&principal),
                "resolve_approval".to_owned(),
                "approval".to_owned(),
                record.approval_id.to_string(),
                AuditOutcome::Success,
                serde_json::json!({ "decision": "rejected" }),
            )
            .await
        {
            Ok(_) => {
                crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
                (StatusCode::OK, Json(record)).into_response()
            }
            Err(err) => runtime_error_response(err),
        },
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn deny_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let approval_id = ApprovalId::new(id);
    if let Err(response) =
        load_approval_visible_to_tenant(state.as_ref(), &tenant_scope, &approval_id).await
    {
        return response;
    }
    let before = crate::handlers::sse::current_event_head(&state).await;
    match state
        .runtime
        .approvals
        .resolve(&approval_id, ApprovalDecision::Rejected)
        .await
    {
        Ok(record) => match state
            .runtime
            .audits
            .record(
                record.project.tenant_id.clone(),
                audit_actor_id(&principal),
                "resolve_approval".to_owned(),
                "approval".to_owned(),
                record.approval_id.to_string(),
                AuditOutcome::Success,
                serde_json::json!({ "decision": "denied" }),
            )
            .await
        {
            Ok(_) => {
                crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
                (StatusCode::OK, Json(record)).into_response()
            }
            Err(err) => runtime_error_response(err),
        },
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn delegate_approval_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<DelegateApprovalRequest>,
) -> impl IntoResponse {
    // delegate() is not part of the ApprovalService trait; return stub.
    let _ = (state, id, body);
    AppApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "not_implemented",
        "approval delegation is not yet implemented",
    )
    .into_response()
}
