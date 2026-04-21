//! Worker register, claim, heartbeat, and fleet handlers.
//!
//! Extracted from `lib.rs` — contains external worker registration, listing,
//! get, suspend, reactivate, fleet overview, task claim, report, and heartbeat.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::http::ListResponse;
use cairn_domain::workers::{ExternalWorkerProgress, ExternalWorkerReport};
use cairn_domain::{ProjectKey, RunId, TaskId, WorkerId};
use cairn_runtime::ExternalWorkerService;

use crate::errors::{
    bad_request_response, now_ms, runtime_error_response, tenant_scope_mismatch_error,
};
use crate::extractors::TenantScope;
use crate::helpers::{build_external_worker_report, scoped_worker};
use crate::state::AppState;

const DEFAULT_TENANT_ID: &str = "default_tenant";
const DEFAULT_WORKSPACE_ID: &str = "default_workspace";
const DEFAULT_PROJECT_ID: &str = "default_project";

// ── DTOs ────────────────────────────────────────────────────────────────────

// PaginationQuery is defined in admin.rs and re-exported via crate::*
use crate::handlers::admin::PaginationQuery;

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct RegisterWorkerRequest {
    pub worker_id: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RegisteredWorkerResponse {
    pub worker_id: String,
    pub registered: bool,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct SuspendWorkerRequest {
    pub reason: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct WorkerClaimRequest {
    pub task_id: String,
    pub lease_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct WorkerReportRouteRequest {
    pub tenant_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub task_id: String,
    pub lease_token: u64,
    pub run_id: Option<String>,
    pub message: Option<String>,
    pub percent: Option<u16>,
    pub outcome: Option<String>,
}

impl WorkerReportRouteRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct WorkerHeartbeatRequest {
    pub task_id: String,
    pub lease_token: u64,
    pub lease_extension_ms: Option<u64>,
    pub tenant_id: Option<String>,
    pub workspace_id: Option<String>,
    pub project_id: Option<String>,
    pub run_id: Option<String>,
    pub message: Option<String>,
    pub percent: Option<u16>,
}

impl WorkerHeartbeatRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
            self.workspace_id.as_deref().unwrap_or(DEFAULT_WORKSPACE_ID),
            self.project_id.as_deref().unwrap_or(DEFAULT_PROJECT_ID),
        )
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

pub(crate) async fn register_worker_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Json(body): Json<RegisterWorkerRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .external_workers
        .register(
            tenant_scope.tenant_id().clone(),
            WorkerId::new(body.worker_id.clone()),
            body.display_name.unwrap_or_else(|| body.worker_id.clone()),
        )
        .await
    {
        Ok(worker) => (
            StatusCode::CREATED,
            Json(RegisteredWorkerResponse {
                worker_id: worker.worker_id.to_string(),
                registered: true,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_workers_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Query(query): Query<PaginationQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .external_workers
        .list(tenant_scope.tenant_id(), query.limit(), query.offset())
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

pub(crate) async fn get_worker_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match scoped_worker(state.as_ref(), tenant_scope.tenant_id(), &id).await {
        Ok(worker) => (StatusCode::OK, Json(worker)).into_response(),
        Err(err) => err.into_response(),
    }
}

pub(crate) async fn suspend_worker_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(_body): Json<SuspendWorkerRequest>,
) -> impl IntoResponse {
    match scoped_worker(state.as_ref(), tenant_scope.tenant_id(), &id).await {
        Ok(_) => {}
        Err(err) => return err.into_response(),
    }

    match state
        .runtime
        .external_workers
        .suspend(&WorkerId::new(id))
        .await
    {
        Ok(worker) => (StatusCode::OK, Json(worker)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn reactivate_worker_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match scoped_worker(state.as_ref(), tenant_scope.tenant_id(), &id).await {
        Ok(_) => {}
        Err(err) => return err.into_response(),
    }

    match state
        .runtime
        .external_workers
        .reactivate(&WorkerId::new(id))
        .await
    {
        Ok(worker) => (StatusCode::OK, Json(worker)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

/// GET /v1/fleet — returns registered external workers with health and task status.
pub(crate) async fn fleet_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
) -> impl IntoResponse {
    use cairn_runtime::{FleetService, FleetServiceImpl};
    let svc = FleetServiceImpl::new(state.runtime.store.clone());
    match svc.fleet_report(tenant_scope.tenant_id(), 200).await {
        Ok(report) => (StatusCode::OK, Json(report)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn worker_claim_task_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(worker_id): Path<String>,
    Json(body): Json<WorkerClaimRequest>,
) -> impl IntoResponse {
    match scoped_worker(state.as_ref(), tenant_scope.tenant_id(), &worker_id).await {
        Ok(_) => {}
        Err(err) => return err.into_response(),
    }

    let task_id = TaskId::new(body.task_id);
    // RFC-011 Phase 2: adapter derives session from the projection.
    match state
        .runtime
        .tasks
        .claim(
            None,
            &task_id,
            worker_id,
            body.lease_duration_ms.unwrap_or(60_000),
        )
        .await
    {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn worker_report_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(worker_id): Path<String>,
    Json(body): Json<WorkerReportRouteRequest>,
) -> impl IntoResponse {
    match scoped_worker(state.as_ref(), tenant_scope.tenant_id(), &worker_id).await {
        Ok(_) => {}
        Err(err) => return err.into_response(),
    }

    if !tenant_scope.is_admin && body.project().tenant_id != *tenant_scope.tenant_id() {
        return tenant_scope_mismatch_error().into_response();
    }

    let report = match build_external_worker_report(
        &worker_id,
        &body.project(),
        &body.task_id,
        body.lease_token,
        body.run_id.as_deref(),
        body.message.clone(),
        body.percent,
        body.outcome.as_deref(),
    ) {
        Ok(report) => report,
        Err(err) => return bad_request_response(err),
    };

    match state.runtime.external_workers.report(report).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn worker_heartbeat_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(worker_id): Path<String>,
    Json(body): Json<WorkerHeartbeatRequest>,
) -> impl IntoResponse {
    match scoped_worker(state.as_ref(), tenant_scope.tenant_id(), &worker_id).await {
        Ok(_) => {}
        Err(err) => return err.into_response(),
    }

    if !tenant_scope.is_admin && body.project().tenant_id != *tenant_scope.tenant_id() {
        return tenant_scope_mismatch_error().into_response();
    }

    let hb_task_id = TaskId::new(body.task_id.clone());
    // RFC-011 Phase 2: adapter derives session from the projection.
    match state
        .runtime
        .tasks
        .heartbeat(
            None,
            &hb_task_id,
            body.lease_extension_ms.unwrap_or(60_000),
        )
        .await
    {
        Ok(task) => {
            let report = ExternalWorkerReport {
                project: body.project(),
                worker_id: WorkerId::new(worker_id),
                run_id: body.run_id.map(RunId::new),
                task_id: TaskId::new(body.task_id),
                lease_token: body.lease_token,
                reported_at_ms: now_ms(),
                progress: Some(ExternalWorkerProgress {
                    message: body.message,
                    percent_milli: body.percent,
                }),
                outcome: None,
            };

            match state.runtime.external_workers.report(report).await {
                Ok(()) => (StatusCode::OK, Json(task)).into_response(),
                Err(err) => runtime_error_response(err),
            }
        }
        Err(err) => runtime_error_response(err),
    }
}
