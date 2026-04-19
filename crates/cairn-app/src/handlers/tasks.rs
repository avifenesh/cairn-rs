//! Task CRUD, dependencies, leasing, and lifecycle HTTP handlers.

use std::sync::Arc;

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::auth::AuthPrincipal;
use cairn_api::http::ApiError;
use cairn_api::http::ListResponse;
use cairn_domain::{
    AuditOutcome, EventEnvelope, EventId, EventSource, ProjectKey, RunId, RunState, RuntimeEvent,
    StateTransition, TaskId, TaskState, TaskStateChanged,
};
use cairn_runtime::AuditService;
use cairn_store::projections::{
    TaskDependencyReadModel, TaskLeaseExpiredReadModel, TaskReadModel, TaskRecord,
};
use cairn_store::EventLog;
use utoipa::ToSchema;

use crate::errors::{
    bad_request_response, parse_task_state, runtime_error_response, store_error_response,
    AppApiError,
};
use crate::extractors::{HasProjectScope, ProjectJson, ProjectScope, TenantScope};
use crate::helpers::resolve_session_for_task_record;
use crate::state::AppState;
#[allow(unused_imports)]
use crate::TaskRecordDoc;
use crate::{
    append_runtime_event, audit_actor_id, current_event_head, publish_runtime_frames_since,
};

// ── Constants ────────────────────────────────────────────────────────────────

/// T6a-C3 helper: load a task and verify it belongs to the caller's tenant.
/// Returns 404 on both missing and cross-tenant so existence doesn't leak.
async fn load_task_visible_to_tenant(
    state: &AppState,
    tenant_scope: &TenantScope,
    task_id: &TaskId,
) -> Result<TaskRecord, axum::response::Response> {
    match state.runtime.tasks.get(task_id).await {
        Ok(Some(task))
            if tenant_scope.is_admin || task.project.tenant_id == *tenant_scope.tenant_id() =>
        {
            Ok(task)
        }
        Ok(_) => Err(
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "task not found").into_response(),
        ),
        Err(err) => Err(runtime_error_response(err)),
    }
}

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct TaskListQuery {
    pub(crate) tenant_id: String,
    pub(crate) workspace_id: String,
    pub(crate) project_id: String,
    pub(crate) run_id: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
}

impl TaskListQuery {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit.unwrap_or(50).min(200)
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

impl HasProjectScope for TaskListQuery {
    fn project(&self) -> ProjectKey {
        TaskListQuery::project(self)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct ClaimTaskRequest {
    pub(crate) worker_id: String,
    pub(crate) lease_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct HeartbeatTaskRequest {
    pub(crate) worker_id: String,
    pub(crate) lease_extension_ms: Option<u64>,
}

#[derive(Clone, Debug, serde::Deserialize, ToSchema)]
#[allow(dead_code)]
pub(crate) struct CreateTaskRequest {
    pub(crate) tenant_id: String,
    pub(crate) workspace_id: String,
    pub(crate) project_id: String,
    pub(crate) task_id: String,
    pub(crate) parent_run_id: Option<String>,
    pub(crate) parent_task_id: Option<String>,
    pub(crate) priority: Option<u8>,
}

impl CreateTaskRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }

    /// SEC-002: reject null bytes, control chars, and oversized ids that
    /// would otherwise flow into FF key builders where a null byte is a
    /// delimiter — mirrors `CreateRunRequest::validate`.
    pub(crate) fn validate(&self) -> Result<(), String> {
        crate::validate::check_all(&[
            crate::validate::require_id("tenant_id", &self.tenant_id),
            crate::validate::require_id("workspace_id", &self.workspace_id),
            crate::validate::require_id("project_id", &self.project_id),
            crate::validate::require_id("task_id", &self.task_id),
            crate::validate::valid_id("parent_run_id", &self.parent_run_id),
            crate::validate::valid_id("parent_task_id", &self.parent_task_id),
        ])
    }
}

impl HasProjectScope for CreateTaskRequest {
    fn project(&self) -> ProjectKey {
        CreateTaskRequest::project(self)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct SetTaskPriorityRequest {
    pub(crate) priority: u8,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct ExpireLeasesResponse {
    pub(crate) expired_count: u32,
    pub(crate) task_ids: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct AddTaskDependencyRequest {
    pub(crate) depends_on_task_id: String,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

pub(crate) async fn list_tasks_handler(
    State(state): State<Arc<AppState>>,
    project_scope: ProjectScope<TaskListQuery>,
) -> impl IntoResponse {
    let query = project_scope.into_inner();
    let state_filter = match query.state.as_deref().map(parse_task_state).transpose() {
        Ok(state_filter) => state_filter,
        Err(err) => return bad_request_response(err),
    };
    let run_id = query.run_id.as_deref().map(RunId::new);
    let limit = query.limit();

    match state
        .runtime
        .store
        .list_tasks_filtered(
            &TaskListQuery::project(&query),
            run_id.as_ref(),
            state_filter,
            limit + 1,
            query.offset(),
        )
        .await
    {
        Ok(mut items) => {
            let has_more = items.len() > limit;
            items.truncate(limit);
            (StatusCode::OK, Json(ListResponse { items, has_more })).into_response()
        }
        Err(err) => store_error_response(err),
    }
}

#[utoipa::path(
    post,
    path = "/v1/tasks",
    tag = "runtime",
    request_body = CreateTaskRequest,
    responses(
        (status = 201, description = "Task created", body = TaskRecordDoc),
        (status = 400, description = "Invalid request", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Parent run not found", body = ApiError),
        (status = 422, description = "Unprocessable entity", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError)
    )
)]
pub(crate) async fn create_task_handler(
    State(state): State<Arc<AppState>>,
    project_scope: ProjectJson<CreateTaskRequest>,
) -> impl IntoResponse {
    let body = project_scope.into_inner();
    // SEC-002: validate ids before they reach FF key builders.
    if let Err(msg) = body.validate() {
        return bad_request_response(msg);
    }
    let project = CreateTaskRequest::project(&body);
    let mut session_id: Option<cairn_domain::SessionId> = None;
    if let Some(parent_run_id) = body.parent_run_id.as_ref().map(RunId::new) {
        match state.runtime.runs.get(&parent_run_id).await {
            Ok(Some(parent_run)) if parent_run.project == project => {
                session_id = Some(parent_run.session_id.clone());
            }
            Ok(Some(_)) | Ok(None) => {
                return AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "parent run not found",
                )
                .into_response();
            }
            Err(err) => return runtime_error_response(err),
        }
    }
    if let Some(parent_task_id) = body.parent_task_id.as_ref().map(TaskId::new) {
        match state.runtime.tasks.get(&parent_task_id).await {
            Ok(Some(parent_task)) if parent_task.project == project => {
                if session_id.is_none() {
                    session_id =
                        resolve_session_for_task_record(state.as_ref(), &parent_task).await;
                }
            }
            Ok(Some(_)) | Ok(None) => {
                return AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "parent task not found",
                )
                .into_response();
            }
            Err(err) => return runtime_error_response(err),
        }
    }
    let before = current_event_head(&state).await;
    match state
        .runtime
        .tasks
        .submit(
            &project,
            session_id.as_ref(),
            TaskId::new(body.task_id.clone()),
            body.parent_run_id.clone().map(RunId::new),
            body.parent_task_id.clone().map(TaskId::new),
            body.priority.unwrap_or(0) as u32,
        )
        .await
    {
        Ok(task) => {
            if let Some(parent_run_id) = task.parent_run_id.clone() {
                match state.runtime.runs.get(&parent_run_id).await {
                    Ok(Some(run)) if run.state == RunState::Pending => {
                        if let Err(err) = append_runtime_event(
                            &state,
                            cairn_domain::RuntimeEvent::RunStateChanged(
                                cairn_domain::RunStateChanged {
                                    project: run.project.clone(),
                                    run_id: run.run_id.clone(),
                                    transition: cairn_domain::StateTransition {
                                        from: Some(RunState::Pending),
                                        to: RunState::Running,
                                    },
                                    failure_class: None,
                                    pause_reason: None,
                                    resume_trigger: None,
                                },
                            ),
                            "run_state_running",
                        )
                        .await
                        {
                            return runtime_error_response(err);
                        }
                    }
                    Ok(_) => {}
                    Err(err) => return runtime_error_response(err),
                }
            }

            publish_runtime_frames_since(&state, before).await;
            (StatusCode::CREATED, Json(task)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn get_task_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.runtime.tasks.get(&TaskId::new(id)).await {
        Ok(Some(task)) if task.project.tenant_id == *tenant_scope.tenant_id() => {
            (StatusCode::OK, Json(task)).into_response()
        }
        Ok(Some(_)) | Ok(None) => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "task not found").into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn add_task_dependency_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<AddTaskDependencyRequest>,
) -> impl IntoResponse {
    let task_id = TaskId::new(id);
    let depends_on = TaskId::new(body.depends_on_task_id);

    // T6a-C3: both tasks must be in the caller's tenant.
    if let Err(resp) = load_task_visible_to_tenant(state.as_ref(), &tenant_scope, &task_id).await {
        return resp;
    }
    if let Err(resp) = load_task_visible_to_tenant(state.as_ref(), &tenant_scope, &depends_on).await
    {
        return resp;
    }

    let before = current_event_head(&state).await;
    match state
        .runtime
        .tasks
        .declare_dependency(&task_id, &depends_on)
        .await
    {
        Ok(record) => {
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::CREATED, Json(record)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_task_dependencies_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let task_id = TaskId::new(id);
    match state.runtime.tasks.get(&task_id).await {
        Ok(Some(task)) if task.project.tenant_id == *tenant_scope.tenant_id() => {
            match TaskDependencyReadModel::list_blocking(state.runtime.store.as_ref(), &task_id)
                .await
            {
                Ok(records) => (StatusCode::OK, Json(records)).into_response(),
                Err(err) => store_error_response(err),
            }
        }
        Ok(Some(_)) | Ok(None) => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "task not found").into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn set_task_priority_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(_body): Json<SetTaskPriorityRequest>,
) -> impl IntoResponse {
    let task_id = TaskId::new(id);
    // T6a-C3 scope + T6a-L7: set_priority isn't implemented in
    // TaskService, so surface 501 rather than lying with 200+record.
    match load_task_visible_to_tenant(state.as_ref(), &tenant_scope, &task_id).await {
        Ok(_) => AppApiError::new(
            StatusCode::NOT_IMPLEMENTED,
            "not_implemented",
            "task priority mutation is not yet implemented",
        )
        .into_response(),
        Err(response) => response,
    }
}

pub(crate) async fn list_expired_tasks_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    match TaskLeaseExpiredReadModel::list_expired(state.runtime.store.as_ref(), now_ms).await {
        Ok(tasks) => (
            StatusCode::OK,
            Json(ListResponse::<TaskRecord> {
                items: tasks,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn expire_task_leases_handler(
    State(state): State<Arc<AppState>>,
    _role: crate::extractors::AdminRoleGuard,
) -> impl IntoResponse {
    // T6a-C3: this is an admin-level operation — it scans every tenant's
    // expired leases and requeues them. Gate on AdminRoleGuard so a
    // non-admin operator can't force cross-tenant requeues.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let expired = match state.runtime.tasks.list_expired_leases(now, 1000).await {
        Ok(e) => e,
        Err(err) => return runtime_error_response(err),
    };

    let mut task_ids: Vec<String> = Vec::new();
    for task in &expired {
        // Requeue each expired task: transition Leased → Queued and clear the lease.
        let event = EventEnvelope::for_runtime_event(
            EventId::new(format!("expire_{}_{now}", task.task_id.as_str())),
            EventSource::Runtime,
            RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: task.project.clone(),
                task_id: task.task_id.clone(),
                transition: StateTransition {
                    from: Some(cairn_domain::TaskState::Leased),
                    to: cairn_domain::TaskState::Queued,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
        );
        if state.runtime.store.append(&[event]).await.is_ok() {
            task_ids.push(task.task_id.to_string());
        }
    }
    let expired_count = task_ids.len() as u32;
    (
        StatusCode::OK,
        Json(ExpireLeasesResponse {
            expired_count,
            task_ids,
        }),
    )
        .into_response()
}

pub(crate) async fn claim_task_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<ClaimTaskRequest>,
) -> impl IntoResponse {
    let task_id = TaskId::new(id);
    let task = match load_task_visible_to_tenant(state.as_ref(), &tenant_scope, &task_id).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };
    let session_id = resolve_session_for_task_record(state.as_ref(), &task).await;

    let before = current_event_head(&state).await;
    match state
        .runtime
        .tasks
        .claim(
            session_id.as_ref(),
            &task_id,
            body.worker_id,
            body.lease_duration_ms.unwrap_or(60_000),
        )
        .await
    {
        Ok(task) => {
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(task)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn heartbeat_task_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<HeartbeatTaskRequest>,
) -> impl IntoResponse {
    let task_id = TaskId::new(id);
    let task = match load_task_visible_to_tenant(state.as_ref(), &tenant_scope, &task_id).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };
    let session_id = resolve_session_for_task_record(state.as_ref(), &task).await;

    let before = current_event_head(&state).await;
    match state
        .runtime
        .tasks
        .heartbeat(
            session_id.as_ref(),
            &task_id,
            body.lease_extension_ms.unwrap_or(60_000),
        )
        .await
    {
        Ok(task) => {
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(task)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn release_task_lease_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let task_id = TaskId::new(id);
    let task = match state.runtime.tasks.get(&task_id).await {
        Ok(Some(task)) if task.project.tenant_id == *tenant_scope.tenant_id() => task,
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "task not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };
    let session_id = resolve_session_for_task_record(state.as_ref(), &task).await;

    let before = current_event_head(&state).await;
    match state
        .runtime
        .tasks
        .release_lease(session_id.as_ref(), &task_id)
        .await
    {
        Ok(task) => {
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(task)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn cancel_task_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let task_id = TaskId::new(id);
    // T6a-C3: tenant scope enforced via helper (replaces the prior
    // get-only check that audited the task's own tenant without auth).
    let task = match load_task_visible_to_tenant(state.as_ref(), &tenant_scope, &task_id).await {
        Ok(t) => t,
        Err(response) => return response,
    };
    let session_id = resolve_session_for_task_record(state.as_ref(), &task).await;

    let before = current_event_head(&state).await;
    match state
        .runtime
        .tasks
        .cancel(session_id.as_ref(), &task_id)
        .await
    {
        Ok(record) => {
            let _ = state
                .runtime
                .audits
                .record(
                    task.project.tenant_id.clone(),
                    audit_actor_id(&principal),
                    "cancel_task".to_owned(),
                    "task".to_owned(),
                    task_id.to_string(),
                    AuditOutcome::Success,
                    serde_json::json!({ "previous_state": format!("{:?}", task.state) }),
                )
                .await;
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(record)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn complete_task_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let before = current_event_head(&state).await;
    let task_id = TaskId::new(id);
    // T6a-C3: tenant scope check.
    let current_task =
        match load_task_visible_to_tenant(state.as_ref(), &tenant_scope, &task_id).await {
            Ok(t) => t,
            Err(response) => return response,
        };
    // Fetch the parent run once: we need its session_id to drive task
    // completion and, if the run becomes eligible, to complete it below.
    let parent_run = match current_task.parent_run_id.as_ref() {
        Some(rid) => state.runtime.runs.get(rid).await.ok().flatten(),
        None => None,
    };
    let session_id = parent_run.as_ref().map(|r| r.session_id.clone());

    if current_task.state == TaskState::Leased {
        if let Err(err) = state
            .runtime
            .tasks
            .start(session_id.as_ref(), &task_id)
            .await
        {
            return runtime_error_response(err);
        }
    }

    match state
        .runtime
        .tasks
        .complete(session_id.as_ref(), &task_id)
        .await
    {
        Ok(task) => {
            // Auto-checkpoint on task_complete is handled inside
            // TaskServiceImpl::complete() to avoid double-checkpoint races.

            if let Some(parent_run_id) = task.parent_run_id.clone() {
                match TaskReadModel::any_non_terminal_children(
                    state.runtime.store.as_ref(),
                    &parent_run_id,
                )
                .await
                {
                    Ok(false) => {
                        if let Some(run) = parent_run.as_ref() {
                            if run.state == RunState::Running {
                                if let Err(err) = state
                                    .runtime
                                    .runs
                                    .complete(&run.session_id, &parent_run_id)
                                    .await
                                {
                                    return runtime_error_response(err);
                                }
                            }
                        }
                    }
                    Ok(true) => {}
                    Err(err) => {
                        tracing::error!("complete_task check non-terminal children failed: {err}");
                        return AppApiError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "internal_error",
                            err.to_string(),
                        )
                        .into_response();
                    }
                }
            }
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(task)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}
