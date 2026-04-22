//! Run HTTP handlers and request/response DTOs.

use std::sync::Arc;

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::auth::AuthPrincipal;
use cairn_api::http::{ApiError, ListResponse};
use cairn_domain::{
    CheckpointId, MailboxMessageId, PauseReason, PauseReasonKind, ProjectKey, ResumeTrigger, RunId,
    RunResumeTarget, RunState, RunStateChanged, RuntimeEvent, SessionId, StateTransition, TaskId,
    WorkspaceRole,
};
use cairn_runtime::{
    MailboxService, NotificationService, RunCostAlertService, RunSlaService, RuntimeError,
};
use cairn_store::projections::{
    AuditLogReadModel, CheckpointReadModel, OperatorInterventionReadModel, PauseScheduleReadModel,
    RecoveryEscalationReadModel, RunCostReadModel, RunReadModel, SessionCostReadModel,
    TaskReadModel,
};
use cairn_store::{EntityRef, EventLog, EventPosition, StoredEvent};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::errors::{
    bad_request_response, now_ms, operator_event_envelope, parse_run_state, runtime_error_response,
    store_error_response, validation_error_response, AppApiError,
};
use crate::extractors::{HasProjectScope, ProjectJson, ProjectScope, TenantCostQuery, TenantScope};
use crate::helpers::{
    build_diagnosis_report, build_run_record_view, build_run_replay_result,
    checkpoint_recorded_position, load_run_visible_to_tenant, working_dir_for_run,
};
use crate::middleware::ensure_workspace_role_for_project;
use crate::sandbox::workspace_error_response;
use crate::state::AppState;
use crate::{
    append_run_intervention_event, current_event_head, event_message, event_type_name,
    persist_run_mode_default, publish_runtime_frames_since, resolve_run_mode_default,
    resolve_run_string_default, PaginationQuery, RunRecordView,
};
#[allow(unused_imports)]
use crate::{RunListResponseDoc, RunRecordDoc};
use cairn_store::projections::{RunRecord, TaskRecord};

// ── Constants ────────────────────────────────────────────────────────────────

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct SpawnSubagentRunResponse {
    pub(crate) parent_run_id: String,
    pub(crate) child_run_id: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RunDetailResponse {
    pub(crate) run: RunRecordView,
    pub(crate) tasks: Vec<TaskRecord>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct AuditEntry {
    #[serde(rename = "type")]
    pub(crate) entry_type: String,
    pub(crate) timestamp_ms: u64,
    pub(crate) description: String,
    pub(crate) actor: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct AuditTrail {
    pub(crate) run_id: String,
    pub(crate) entries: Vec<AuditEntry>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunInterventionResponse {
    pub(crate) ok: bool,
    pub(crate) run: Option<RunRecord>,
    pub(crate) message_id: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // T6a-H10: response shape widened to include `failures: Vec`.
pub(crate) struct ScheduledResumeProcessResponse {
    pub(crate) resumed_count: usize,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct RunReplayQuery {
    pub(crate) from_position: Option<u64>,
    pub(crate) to_position: Option<u64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct StalledRunsQuery {
    pub(crate) minutes: Option<u64>,
}

impl StalledRunsQuery {
    pub(crate) fn stale_after_ms(&self) -> u64 {
        self.minutes.unwrap_or(30).saturating_mul(60_000)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct ReplayToCheckpointQuery {
    pub(crate) checkpoint_id: String,
}

/// Paginated event query params: cursor (exclusive lower bound) + limit.
#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct EventsPageQuery {
    pub(crate) cursor: Option<u64>,
    /// Alias for cursor (legacy/test compatibility): return events as a plain array.
    pub(crate) from: Option<u64>,
    pub(crate) limit: Option<usize>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct EventSummary {
    pub(crate) position: u64,
    pub(crate) event_type: String,
    pub(crate) occurred_at_ms: u64,
    pub(crate) description: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct EventsPage {
    pub(crate) events: Vec<EventSummary>,
    pub(crate) next_cursor: Option<u64>,
    pub(crate) has_more: bool,
}

#[derive(Clone, Debug, serde::Deserialize, ToSchema)]
pub(crate) struct RunListQuery {
    pub(crate) tenant_id: String,
    pub(crate) workspace_id: String,
    pub(crate) project_id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
}

impl RunListQuery {
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

impl HasProjectScope for RunListQuery {
    fn project(&self) -> ProjectKey {
        RunListQuery::project(self)
    }
}

#[derive(Clone, Debug, serde::Deserialize, ToSchema)]
pub(crate) struct CreateRunRequest {
    pub(crate) tenant_id: String,
    pub(crate) workspace_id: String,
    pub(crate) project_id: String,
    pub(crate) session_id: String,
    pub(crate) run_id: String,
    pub(crate) parent_run_id: Option<String>,
    /// RFC 018: execution mode (direct/plan/execute).
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub(crate) mode: Option<cairn_domain::decisions::RunMode>,
}

impl CreateRunRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }

    /// SEC-002: reject control-character / empty / oversized inputs at
    /// the HTTP boundary before any id flows into FF's key builders
    /// (where a null-byte is a delimiter — see id_map.rs F02 fix). Must
    /// be called explicitly by every handler that consumes this struct;
    /// the `project()` accessor intentionally stays infallible so it can
    /// continue serving as the `HasProjectScope` impl.
    pub(crate) fn validate(&self) -> Result<(), String> {
        crate::validate::check_all(&[
            crate::validate::require_id("tenant_id", &self.tenant_id),
            crate::validate::require_id("workspace_id", &self.workspace_id),
            crate::validate::require_id("project_id", &self.project_id),
            crate::validate::require_id("session_id", &self.session_id),
            crate::validate::require_id("run_id", &self.run_id),
            crate::validate::valid_id("parent_run_id", &self.parent_run_id),
        ])
    }
}

impl HasProjectScope for CreateRunRequest {
    fn project(&self) -> ProjectKey {
        CreateRunRequest::project(self)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct SpawnSubagentRunRequest {
    pub(crate) session_id: String,
    pub(crate) parent_task_id: Option<String>,
    pub(crate) child_task_id: Option<String>,
    pub(crate) child_run_id: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct PauseRunRequest {
    #[serde(alias = "kind")]
    pub(crate) reason_kind: Option<PauseReasonKind>,
    pub(crate) detail: Option<String>,
    pub(crate) actor: Option<String>,
    pub(crate) resume_after_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct ResumeRunRequest {
    pub(crate) trigger: Option<ResumeTrigger>,
    pub(crate) target: Option<RunResumeTarget>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunInterventionAction {
    ForceComplete,
    ForceFail,
    ForceRestart,
    InjectMessage,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct RunInterventionRequest {
    pub(crate) action: RunInterventionAction,
    pub(crate) reason: String,
    pub(crate) message_body: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SetRunCostAlertRequest {
    /// T6a-C2: tenant_id is accepted in the body for schema compat but
    /// ignored — the handler uses the resolved run's tenant_id instead.
    #[serde(default, rename = "tenant_id")]
    pub(crate) _tenant_id_deprecated: Option<String>,
    pub(crate) threshold_micros: u64,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SetRunSlaRequest {
    /// T6a-C2: tenant_id is accepted in the body for schema compat but
    /// ignored — the handler uses the resolved run's tenant_id instead.
    #[serde(default, rename = "tenant_id")]
    pub(crate) _tenant_id_deprecated: Option<String>,
    pub(crate) target_completion_ms: u64,
    #[serde(default = "default_alert_pct")]
    pub(crate) alert_at_percent: u8,
}

fn default_alert_pct() -> u8 {
    80
}

#[derive(serde::Deserialize)]
pub(crate) struct OrchestrateRequest {
    #[serde(default)]
    pub(crate) goal: Option<String>,
    #[serde(default)]
    pub(crate) model_id: Option<String>,
    #[serde(default)]
    pub(crate) max_iterations: Option<u32>,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    /// RFC 018: execution mode override for this orchestration.
    #[serde(default)]
    pub(crate) mode: Option<cairn_domain::decisions::RunMode>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/runs",
    tag = "runtime",
    responses(
        (status = 200, description = "Runs listed", body = RunListResponseDoc),
        (status = 400, description = "Invalid request", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError)
    )
)]
pub(crate) async fn list_runs_handler(
    State(state): State<Arc<AppState>>,
    project_scope: ProjectScope<RunListQuery>,
) -> impl IntoResponse {
    let query = project_scope.into_inner();
    let status_filter = match query.status.as_deref().map(parse_run_state).transpose() {
        Ok(status_filter) => status_filter,
        Err(err) => return bad_request_response(err),
    };
    let session_id = query.session_id.as_deref().map(SessionId::new);
    let limit = query.limit();
    match state
        .runtime
        .store
        .list_runs_filtered(
            &RunListQuery::project(&query),
            session_id.as_ref(),
            status_filter,
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

pub(crate) async fn get_run_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);
    match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(run)) => {
            let run = build_run_record_view(state.as_ref(), run).await;
            match TaskReadModel::list_by_parent_run(
                state.runtime.store.as_ref(),
                &run.run.run_id,
                200,
            )
            .await
            {
                Ok(tasks) => {
                    (StatusCode::OK, Json(RunDetailResponse { run, tasks })).into_response()
                }
                Err(err) => store_error_response(err),
            }
        }
        Ok(None) => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found").into_response()
        }
        Err(response) => response,
    }
}

#[utoipa::path(
    post,
    path = "/v1/runs",
    tag = "runtime",
    request_body = CreateRunRequest,
    responses(
        (status = 201, description = "Run created", body = RunRecordDoc),
        (status = 400, description = "Invalid request", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Session not found", body = ApiError),
        (status = 422, description = "Unprocessable entity", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError)
    )
)]
pub(crate) async fn create_run_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    trace_id: Option<Extension<crate::middleware::TraceId>>,
    project_scope: ProjectJson<CreateRunRequest>,
) -> impl IntoResponse {
    let body = project_scope.into_inner();
    // SEC-002: validate tenant / workspace / project / session / run ids
    // before they flow through FF — null bytes, newlines, and oversized
    // fields must return 422, not propagate into Valkey key builders.
    if let Err(msg) = body.validate() {
        return validation_error_response(msg);
    }
    let project = CreateRunRequest::project(&body);
    if let Err(response) = ensure_workspace_role_for_project(
        state.as_ref(),
        &principal,
        &project,
        WorkspaceRole::Member,
    )
    .await
    {
        return response;
    }
    let session_id = SessionId::new(body.session_id.clone());
    match state.runtime.sessions.get(&session_id).await {
        Ok(Some(session)) if session.project == project => {}
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "session not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    }
    if let Some(parent_run_id) = body.parent_run_id.as_ref().map(RunId::new) {
        match state.runtime.runs.get(&parent_run_id).await {
            Ok(Some(parent_run)) if parent_run.project == project => {}
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
    let before = current_event_head(&state).await;
    // RFC 011: if the request arrived with an `x-trace-id` header the
    // middleware put it on extensions as a `TraceId`. Thread it through
    // to Fabric so the emitted `RunCreated` envelope's correlation_id
    // matches the trace id, making `GET /v1/trace/:id` non-empty.
    let correlation_id = trace_id.map(|Extension(t)| t.as_str().to_owned());
    let start_result = match correlation_id.as_deref() {
        Some(corr) if !corr.is_empty() => {
            state
                .runtime
                .runs
                .start_with_correlation(
                    &project,
                    &session_id,
                    RunId::new(body.run_id),
                    body.parent_run_id.map(RunId::new),
                    corr,
                )
                .await
        }
        _ => {
            state
                .runtime
                .runs
                .start(
                    &project,
                    &session_id,
                    RunId::new(body.run_id),
                    body.parent_run_id.map(RunId::new),
                )
                .await
        }
    };
    match start_result {
        Ok(run) => {
            if let Some(mode) = body.mode.as_ref() {
                if let Err(err) =
                    persist_run_mode_default(state.as_ref(), &project, &run.run_id, mode).await
                {
                    return runtime_error_response(err);
                }
            }
            publish_runtime_frames_since(&state, before).await;
            let view = build_run_record_view(state.as_ref(), run).await;
            (StatusCode::CREATED, Json(view)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_stalled_runs_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Query(query): Query<StalledRunsQuery>,
) -> impl IntoResponse {
    let stale_after_ms = query.stale_after_ms();
    let running_runs =
        match RunReadModel::list_by_state(state.runtime.store.as_ref(), RunState::Running, 10_000)
            .await
        {
            Ok(runs) => runs,
            Err(err) => return store_error_response(err),
        };

    let mut items = Vec::new();
    for run in running_runs {
        if run.project.tenant_id != *tenant_scope.tenant_id() {
            continue;
        }

        match build_diagnosis_report(state.as_ref(), &run, stale_after_ms).await {
            Ok((report, true)) => items.push(report),
            Ok((_report, false)) => {}
            Err(err) => return store_error_response(err),
        }
    }

    (
        StatusCode::OK,
        Json(ListResponse {
            items,
            has_more: false,
        }),
    )
        .into_response()
}

pub(crate) async fn list_escalated_runs_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
) -> impl IntoResponse {
    match RecoveryEscalationReadModel::list_by_tenant(
        state.runtime.store.as_ref(),
        tenant_scope.tenant_id(),
    )
    .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse::<cairn_domain::recovery::RecoveryEscalation> {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn diagnose_run_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    match build_diagnosis_report(state.as_ref(), &run, 30 * 60_000).await {
        Ok((report, _)) => (StatusCode::OK, Json(report)).into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn get_run_audit_trail_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let run_id = RunId::new(id.clone());

    // Validate run exists and belongs to tenant
    match state.runtime.runs.get(&run_id).await {
        Ok(Some(run)) if run.project.tenant_id == *tenant_scope.tenant_id() => {}
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    }

    // Read all events for this run from the event log
    let stored_events = match state
        .runtime
        .store
        .read_by_entity(&EntityRef::Run(run_id.clone()), None, 1000)
        .await
    {
        Ok(events) => events,
        Err(err) => return store_error_response(err),
    };

    let mut entries: Vec<AuditEntry> = Vec::new();
    for stored in &stored_events {
        entries.push(AuditEntry {
            entry_type: "event".to_owned(),
            timestamp_ms: stored.stored_at,
            description: event_message(&stored.envelope.payload),
            actor: None,
        });
        // Synthesize an initial-state entry right after RunCreated
        if matches!(&stored.envelope.payload, RuntimeEvent::RunCreated(_)) {
            entries.push(AuditEntry {
                entry_type: "event".to_owned(),
                timestamp_ms: stored.stored_at,
                description: format!("Run {} entered state Pending", run_id.as_str()),
                actor: None,
            });
        }
    }

    // Read audit log entries for this run
    let audit_logs = match AuditLogReadModel::list_by_resource(
        state.runtime.store.as_ref(),
        "run",
        run_id.as_str(),
    )
    .await
    {
        Ok(logs) => logs,
        Err(err) => return store_error_response(err),
    };

    entries.extend(audit_logs.into_iter().map(|entry| AuditEntry {
        entry_type: "audit".to_owned(),
        timestamp_ms: entry.occurred_at_ms,
        description: entry.action.clone(),
        actor: Some(entry.actor_id.clone()),
    }));

    entries.sort_by_key(|e| e.timestamp_ms);

    (
        StatusCode::OK,
        Json(AuditTrail {
            run_id: id,
            entries,
        }),
    )
        .into_response()
}

pub(crate) async fn list_run_events_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Query(query): Query<EventsPageQuery>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    // `from` is a legacy param: treat it as a minimum position filter and return a plain array.
    let use_legacy_array = query.from.is_some() && query.cursor.is_none();
    let cursor = query.cursor.or(query.from).map(EventPosition);

    // Fetch one extra to detect whether more pages exist
    let fetched = match state
        .runtime
        .store
        .read_by_entity(&EntityRef::Run(run.run_id.clone()), cursor, limit + 1)
        .await
    {
        Ok(events) => events,
        Err(err) => return store_error_response(err),
    };

    let has_more = fetched.len() > limit;
    let page: Vec<StoredEvent> = fetched.into_iter().take(limit).collect();
    let next_cursor = if has_more {
        page.last().map(|e| e.position.0)
    } else {
        None
    };

    let events: Vec<EventSummary> = page
        .into_iter()
        .map(|e| EventSummary {
            position: e.position.0,
            event_type: event_type_name(&e.envelope.payload).to_owned(),
            occurred_at_ms: e.stored_at,
            description: event_message(&e.envelope.payload),
        })
        .collect();

    if use_legacy_array {
        // Legacy `from=N` callers expect a plain JSON array of event summaries.
        return (StatusCode::OK, Json(events)).into_response();
    }

    (
        StatusCode::OK,
        Json(EventsPage {
            events,
            next_cursor,
            has_more,
        }),
    )
        .into_response()
}

pub(crate) async fn replay_run_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Query(query): Query<RunReplayQuery>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);
    let run = match state.runtime.runs.get(&run_id).await {
        Ok(Some(run)) if run.project.tenant_id == *tenant_scope.tenant_id() => run,
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };

    if let (Some(from), Some(to)) = (query.from_position, query.to_position) {
        if to < from {
            return validation_error_response("to_position must be >= from_position");
        }
    }

    match build_run_replay_result(
        state.as_ref(),
        &run.run_id,
        query.from_position,
        query.to_position,
    )
    .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn replay_run_to_checkpoint_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Query(query): Query<ReplayToCheckpointQuery>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);
    let run = match state.runtime.runs.get(&run_id).await {
        Ok(Some(run))
            if tenant_scope.is_admin || run.project.tenant_id == *tenant_scope.tenant_id() =>
        {
            run
        }
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };

    let checkpoint_id = CheckpointId::new(query.checkpoint_id);
    let checkpoint =
        match CheckpointReadModel::get(state.runtime.store.as_ref(), &checkpoint_id).await {
            Ok(Some(checkpoint)) if checkpoint.run_id == run.run_id => checkpoint,
            Ok(Some(_)) | Ok(None) => {
                return AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "checkpoint not found for run",
                )
                .into_response();
            }
            Err(err) => return store_error_response(err),
        };

    let checkpoint_position = match checkpoint_recorded_position(
        state.runtime.store.as_ref(),
        &checkpoint.checkpoint_id,
        &run.run_id,
    )
    .await
    {
        Ok(Some(position)) => position,
        Ok(None) => {
            return AppApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "checkpoint event not found",
            )
            .into_response();
        }
        Err(err) => return store_error_response(err),
    };

    match build_run_replay_result(
        state.as_ref(),
        &run.run_id,
        None,
        Some(checkpoint_position.0),
    )
    .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn list_run_interventions_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Query(query): Query<PaginationQuery>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);
    match state.runtime.runs.get(&run_id).await {
        Ok(Some(run)) if run.project.tenant_id == *tenant_scope.tenant_id() => {}
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    }

    match OperatorInterventionReadModel::list_by_run(
        state.runtime.store.as_ref(),
        &run_id,
        query.limit(),
        query.offset(),
    )
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
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn intervene_run_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<RunInterventionRequest>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);
    let run = match state.runtime.runs.get(&run_id).await {
        Ok(Some(run)) if run.project.tenant_id == *tenant_scope.tenant_id() => run,
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };

    let before = current_event_head(&state).await;
    match body.action {
        RunInterventionAction::ForceComplete => {
            match state.runtime.runs.complete(&run.session_id, &run_id).await {
                Ok(updated_run) => {
                    if let Err(err) = append_run_intervention_event(
                        &state,
                        &run_id,
                        tenant_scope.tenant_id(),
                        "force_complete",
                        &body.reason,
                    )
                    .await
                    {
                        return store_error_response(err);
                    }
                    publish_runtime_frames_since(&state, before).await;
                    (
                        StatusCode::OK,
                        Json(RunInterventionResponse {
                            ok: true,
                            run: Some(updated_run),
                            message_id: None,
                        }),
                    )
                        .into_response()
                }
                Err(err) => runtime_error_response(err),
            }
        }
        RunInterventionAction::ForceFail => {
            let events = vec![
                operator_event_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: run.project.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(run.state),
                        to: RunState::Failed,
                    },
                    failure_class: Some(cairn_domain::FailureClass::ExecutionError),
                    pause_reason: None,
                    resume_trigger: None,
                })),
                operator_event_envelope(RuntimeEvent::OperatorIntervention(
                    cairn_domain::OperatorIntervention {
                        run_id: Some(run_id.clone()),
                        tenant_id: tenant_scope.tenant_id().clone(),
                        action: "force_fail".to_owned(),
                        reason: body.reason,
                        intervened_at_ms: now_ms(),
                    },
                )),
            ];
            match state.runtime.store.append(&events).await {
                Ok(_) => {
                    // RFC 008: notify any operators subscribed to run.failed.
                    let _ = state
                        .runtime
                        .notifications
                        .notify_if_applicable(
                            tenant_scope.tenant_id(),
                            "run.failed",
                            serde_json::json!({ "run_id": run_id.as_str() }),
                        )
                        .await;
                    match state.runtime.runs.get(&run_id).await {
                        Ok(Some(updated_run)) => {
                            publish_runtime_frames_since(&state, before).await;
                            (
                                StatusCode::OK,
                                Json(RunInterventionResponse {
                                    ok: true,
                                    run: Some(updated_run),
                                    message_id: None,
                                }),
                            )
                                .into_response()
                        }
                        Ok(None) => AppApiError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "internal_error",
                            "run not found after intervention",
                        )
                        .into_response(),
                        Err(err) => runtime_error_response(err),
                    }
                }
                Err(err) => store_error_response(err),
            }
        }
        RunInterventionAction::ForceRestart => {
            if !run.state.is_terminal() {
                return validation_error_response("force_restart requires a terminal run state");
            }

            let events = vec![
                operator_event_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: run.project.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(run.state),
                        to: RunState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: Some(ResumeTrigger::OperatorResume),
                })),
                operator_event_envelope(RuntimeEvent::OperatorIntervention(
                    cairn_domain::OperatorIntervention {
                        run_id: Some(run_id.clone()),
                        tenant_id: tenant_scope.tenant_id().clone(),
                        action: "force_restart".to_owned(),
                        reason: body.reason,
                        intervened_at_ms: now_ms(),
                    },
                )),
            ];
            match state.runtime.store.append(&events).await {
                Ok(_) => match state.runtime.runs.get(&run_id).await {
                    Ok(Some(updated_run)) => {
                        publish_runtime_frames_since(&state, before).await;
                        (
                            StatusCode::OK,
                            Json(RunInterventionResponse {
                                ok: true,
                                run: Some(updated_run),
                                message_id: None,
                            }),
                        )
                            .into_response()
                    }
                    Ok(None) => AppApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal_error",
                        "run not found after intervention",
                    )
                    .into_response(),
                    Err(err) => runtime_error_response(err),
                },
                Err(err) => store_error_response(err),
            }
        }
        RunInterventionAction::InjectMessage => {
            let Some(message_body) = body.message_body else {
                return validation_error_response("inject_message requires message_body");
            };

            // T6a-H11: reject message injection into a terminal run. A
            // Completed/Failed/Canceled run has no consumer for the
            // mailbox row, so the write would dangle forever.
            if run.state.is_terminal() {
                return AppApiError::new(
                    StatusCode::CONFLICT,
                    "run_terminal",
                    format!(
                        "cannot inject message into run in terminal state {:?}",
                        run.state
                    ),
                )
                .into_response();
            }

            let message_id = MailboxMessageId::new(format!("msg_intervention_{}", Uuid::new_v4()));
            match state
                .runtime
                .mailbox
                .append(
                    &run.project,
                    message_id.clone(),
                    Some(run_id.clone()),
                    None,
                    message_body,
                    None,
                    0,
                )
                .await
            {
                Ok(_) => {
                    if let Err(err) = append_run_intervention_event(
                        &state,
                        &run_id,
                        tenant_scope.tenant_id(),
                        "inject_message",
                        &body.reason,
                    )
                    .await
                    {
                        return store_error_response(err);
                    }
                    publish_runtime_frames_since(&state, before).await;
                    (
                        StatusCode::OK,
                        Json(RunInterventionResponse {
                            ok: true,
                            run: None,
                            message_id: Some(message_id.to_string()),
                        }),
                    )
                        .into_response()
                }
                Err(err) => runtime_error_response(err),
            }
        }
    }
}

/// `POST /v1/runs/:id/cancel` -- cancel a run mid-execution.
///
/// Transitions the run to `Canceled` state and updates the parent session.
pub(crate) async fn cancel_run_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let run_id = RunId::new(&id);

    // T6a-C2: verify tenant scope before any mutation.
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    let before = current_event_head(&state).await;
    match state.runtime.runs.cancel(&run.session_id, &run_id).await {
        Ok(record) => {
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(record)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

/// `POST /v1/runs/:id/claim` — activate the run's FF execution lease.
///
/// Required before `enter_waiting_approval`, `pause`, or any other
/// FCALL that rejects non-active executions (see
/// `RunService::claim` docstring for the full semantics). On the
/// Fabric path this walks `ff_issue_claim_grant` +
/// `ff_claim_execution` (with `ff_claim_resumed_execution` dispatch
/// when the execution is resuming from a prior suspension). On the
/// in-memory courtesy path this is a no-op that returns the current
/// record — there's no lease to activate.
///
/// **NOT idempotent.** Re-claiming an already-active run fails at
/// FF's grant gate with `execution_not_eligible` and surfaces as a
/// 500 here. Callers must claim once per lifecycle. See
/// `RunService::claim` docstring.
///
/// Get-first is belt-and-suspenders against projection staleness —
/// `FabricRunServiceAdapter::claim` already delegates through
/// `resolve_run_project`, which maps missing-in-store to
/// `RuntimeError::NotFound` → 404. Keeping the explicit lookup here
/// avoids relying on that transitive mapping and isolates the 404
/// response from any future change in the adapter layer.
///
/// No request body: runs are not worker-pulled, so the caller never
/// advertises worker identity through this endpoint (unlike
/// `POST /v1/tasks/:id/claim`, which takes `worker_id` +
/// `lease_duration_ms`). Fabric uses
/// `FabricConfig::worker_instance_id` + `lease_ttl_ms` from the
/// process config.
pub(crate) async fn claim_run_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let run_id = RunId::new(&id);

    // T6a-C2: tenant scope + explicit 404 before the adapter call so the
    // 404 path is isolated from future adapter changes.
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    let before = current_event_head(&state).await;
    match state.runtime.runs.claim(&run.session_id, &run_id).await {
        Ok(record) => {
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(record)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn pause_run_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<PauseRunRequest>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);

    // T6a-C2: tenant scope check before any mutation.
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    let before = current_event_head(&state).await;
    let reason = PauseReason {
        kind: body.reason_kind.unwrap_or(PauseReasonKind::OperatorPause),
        detail: body.detail,
        resume_after_ms: body.resume_after_ms,
        actor: body.actor,
    };

    match state
        .runtime
        .runs
        .pause(&run.session_id, &run_id, reason)
        .await
    {
        Ok(run) => {
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(run)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn resume_run_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<ResumeRunRequest>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);

    // T6a-C2: tenant scope check before any mutation.
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    let before = current_event_head(&state).await;
    match state
        .runtime
        .runs
        .resume(
            &run.session_id,
            &run_id,
            body.trigger.unwrap_or(ResumeTrigger::OperatorResume),
            body.target.unwrap_or(RunResumeTarget::Running),
        )
        .await
    {
        Ok(run) => {
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(run)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn get_run_cost_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let run_id = RunId::new(id.clone());
    match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(_run)) => {
            match RunCostReadModel::get_run_cost(state.runtime.store.as_ref(), &run_id).await {
                Ok(Some(record)) => (StatusCode::OK, Json(record)).into_response(),
                Ok(None) => {
                    // Return a zero-valued cost record instead of 404 when no cost data exists.
                    (
                        StatusCode::OK,
                        Json(cairn_domain::providers::RunCostRecord {
                            run_id: RunId::new(id),
                            total_cost_micros: 0,
                            total_tokens_in: 0,
                            total_tokens_out: 0,
                            provider_calls: 0,
                            token_in: 0,
                            token_out: 0,
                        }),
                    )
                        .into_response()
                }
                Err(err) => store_error_response(err),
            }
        }
        Ok(None) => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found").into_response()
        }
        Err(response) => response,
    }
}

pub(crate) async fn set_run_cost_alert_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<SetRunCostAlertRequest>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);

    // T6a-C2: resolve the run under tenant scope and use the run's
    // actual tenant_id — not a body-supplied one, which lets callers
    // forge cross-tenant alerts.
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    match state
        .runtime
        .run_cost_alerts
        .set_alert(run_id, run.project.tenant_id.clone(), body.threshold_micros)
        .await
    {
        Ok(()) => (StatusCode::CREATED, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_run_cost_alerts_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
) -> impl IntoResponse {
    match state
        .runtime
        .run_cost_alerts
        .list_triggered_by_tenant(tenant_scope.tenant_id())
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

pub(crate) async fn set_run_sla_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<SetRunSlaRequest>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);

    // T6a-C2: tenant scope + use run's actual tenant_id.
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    match state
        .runtime
        .run_sla
        .set_sla(
            run_id,
            run.project.tenant_id.clone(),
            body.target_completion_ms,
            body.alert_at_percent,
        )
        .await
    {
        Ok(config) => (StatusCode::CREATED, Json(config)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn get_run_sla_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);

    // T6a-C2: tenant scope before the read.
    match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    }

    match state.runtime.run_sla.check_sla(&run_id).await {
        Ok(status) => (StatusCode::OK, Json(status)).into_response(),
        Err(RuntimeError::NotFound { .. }) => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "SLA not configured for run",
        )
        .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_sla_breached_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
) -> impl IntoResponse {
    match state
        .runtime
        .run_sla
        .list_breached_by_tenant(tenant_scope.tenant_id())
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse::<cairn_domain::sla::SlaBreach> {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_due_run_resumes_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
) -> impl IntoResponse {
    match PauseScheduleReadModel::list_due(state.runtime.store.as_ref(), now_ms()).await {
        Ok(due) => {
            let mut items = Vec::new();
            for record in due {
                if record.project.tenant_id != *tenant_scope.tenant_id() {
                    continue;
                }
                match state.runtime.runs.get(&record.run_id).await {
                    Ok(Some(run)) if run.state == RunState::Paused => items.push(run),
                    Ok(_) => {}
                    Err(err) => return runtime_error_response(err),
                }
            }
            (
                StatusCode::OK,
                Json(ListResponse {
                    items,
                    has_more: false,
                }),
            )
                .into_response()
        }
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn process_scheduled_run_resumes_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
) -> impl IntoResponse {
    let due = match PauseScheduleReadModel::list_due(state.runtime.store.as_ref(), now_ms()).await {
        Ok(due) => due,
        Err(err) => return store_error_response(err),
    };

    let before = current_event_head(&state).await;
    let mut resumed_count = 0usize;
    // T6a-H10: aggregate per-run failures rather than short-circuiting.
    // Bailing mid-loop leaves already-resumed runs without a published
    // SSE frame and leaves the caller guessing about partial success.
    let mut failures: Vec<serde_json::Value> = Vec::new();
    for record in due {
        if record.project.tenant_id != *tenant_scope.tenant_id() {
            continue;
        }
        let session_id = match state.runtime.runs.get(&record.run_id).await {
            Ok(Some(run)) => run.session_id,
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(
                    run_id = %record.run_id,
                    error = %err,
                    "failed to load run for scheduled resume; skipping",
                );
                continue;
            }
        };
        match state
            .runtime
            .runs
            .resume(
                &session_id,
                &record.run_id,
                ResumeTrigger::ResumeAfterTimer,
                RunResumeTarget::Running,
            )
            .await
        {
            Ok(_) => resumed_count += 1,
            Err(RuntimeError::InvalidTransition { .. }) | Err(RuntimeError::NotFound { .. }) => {
                // Non-fatal per-run skip: run moved to terminal state
                // or disappeared between list_due and resume. Ignored
                // silently to match the prior contract.
            }
            Err(err) => {
                tracing::warn!(
                    run_id = %record.run_id,
                    error = %err,
                    "scheduled resume failed — continuing with remaining runs"
                );
                failures.push(serde_json::json!({
                    "run_id": record.run_id.to_string(),
                    "error": err.to_string(),
                }));
            }
        }
    }
    // Always publish whatever did succeed, even on partial failure.
    if resumed_count > 0 {
        publish_runtime_frames_since(&state, before).await;
    }
    // Keep camelCase `resumedCount` for backward compat with the existing
    // UI + integration test. Add `failures` as a new field so callers can
    // opt in to per-run error visibility without breaking old parsers.
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "resumedCount": resumed_count,
            "failures": failures,
        })),
    )
        .into_response()
}

pub(crate) async fn spawn_subagent_run_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<SpawnSubagentRunRequest>,
) -> impl IntoResponse {
    let parent_run_id = RunId::new(id);
    let parent_run = match state.runtime.runs.get(&parent_run_id).await {
        Ok(Some(run)) if run.project.tenant_id == *tenant_scope.tenant_id() => run,
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };

    let child_session_id = SessionId::new(body.session_id);
    match state.runtime.sessions.get(&child_session_id).await {
        Ok(Some(session)) if session.project == parent_run.project => {}
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "session not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    }

    let _child_task_id = body
        .child_task_id
        .map(TaskId::new)
        .unwrap_or_else(|| TaskId::new(format!("task_subagent_{}", Uuid::new_v4())));
    let child_run_id = body
        .child_run_id
        .map(RunId::new)
        .unwrap_or_else(|| RunId::new(format!("run_subagent_{}", Uuid::new_v4())));
    let before = current_event_head(&state).await;
    match state
        .runtime
        .runs
        .spawn_subagent(
            &parent_run.project,
            parent_run_id.clone(),
            &child_session_id,
            Some(child_run_id),
        )
        .await
    {
        Ok(child_run) => {
            publish_runtime_frames_since(&state, before).await;
            (
                StatusCode::CREATED,
                Json(SpawnSubagentRunResponse {
                    parent_run_id: parent_run_id.to_string(),
                    child_run_id: child_run.run_id.to_string(),
                }),
            )
                .into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_child_runs_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Query(query): Query<PaginationQuery>,
) -> impl IntoResponse {
    let parent_run_id = RunId::new(id);
    let parent_run = match state.runtime.runs.get(&parent_run_id).await {
        Ok(Some(run)) if run.project.tenant_id == *tenant_scope.tenant_id() => run,
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };

    match state
        .runtime
        .runs
        .list_child_runs(&parent_run.run_id, query.limit())
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

// ── Orchestrator entry point ──────────────────────────────────────────────

/// POST /v1/runs/:id/orchestrate -- trigger the GATHER -> DECIDE -> EXECUTE loop.
pub(crate) async fn orchestrate_run_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(run_id_str): Path<String>,
    Json(body): Json<OrchestrateRequest>,
) -> impl IntoResponse {
    use cairn_domain::RunId;
    use cairn_orchestrator::{
        LlmDecidePhase, LoopConfig, LoopTermination, OrchestrationContext, OrchestratorLoop,
        RuntimeExecutePhase, StandardGatherPhase,
    };
    use cairn_runtime::services::{
        ApprovalServiceImpl, CheckpointServiceImpl, MailboxServiceImpl, ToolInvocationServiceImpl,
    };
    use cairn_tools::{
        BuiltinToolRegistry, CalculateTool, CancelTaskTool, CreateTaskTool, FileReadTool,
        FileWriteTool, GetApprovalsTool, GetRunTool, GetTaskTool, GitOperationsTool, GlobFindTool,
        GraphQueryTool, GrepSearchTool, HttpRequestTool, JsonExtractTool, ListRunsTool,
        MemorySearchTool, MemoryStoreTool, NotificationSink, NotifyOperatorTool,
        ResolveApprovalTool, ScheduleTaskTool, ScratchPadTool, SearchEventsTool, ShellExecTool,
        SummarizeTextTool, ToolSearchTool, WaitForTaskTool, WebFetchTool,
    };

    let run_id = RunId::new(run_id_str);

    // T6a-C2: tenant scope MUST gate orchestration — this kicks off LLM calls
    // and burns provider budget. Cross-tenant orchestrate is a budget DoS.
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    // Transition run to Running if it's still Pending
    if run.state == cairn_domain::RunState::Pending {
        use cairn_domain::{RunState, RunStateChanged, RuntimeEvent, StateTransition};
        use cairn_runtime::make_envelope;
        let evt = make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
            project: run.project.clone(),
            run_id: run.run_id.clone(),
            transition: StateTransition {
                from: Some(RunState::Pending),
                to: RunState::Running,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }));
        if let Err(e) = state.runtime.store.append(&[evt]).await {
            tracing::warn!("failed to transition run to running: {e}");
        }
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let working_dir = match working_dir_for_run(state.as_ref(), &run).await {
        Ok(path) => path,
        Err(err) => return workspace_error_response(err),
    };
    let default_goal =
        resolve_run_string_default(state.as_ref(), &run.project, &run.run_id, "goal").await;
    let default_agent_role =
        resolve_run_string_default(state.as_ref(), &run.project, &run.run_id, "agent_role").await;
    let default_run_mode =
        resolve_run_mode_default(state.as_ref(), &run.project, &run.run_id).await;

    let ctx = OrchestrationContext {
        project: run.project.clone(),
        session_id: run.session_id.clone(),
        run_id: run.run_id.clone(),
        task_id: None,
        iteration: 0,
        goal: body
            .goal
            .or(default_goal)
            .unwrap_or_else(|| "Execute the run objective.".to_owned()),
        agent_type: run
            .agent_role_id
            .or(default_agent_role)
            .unwrap_or_else(|| "orchestrator".to_owned()),
        run_started_at_ms: now_ms,
        working_dir: working_dir.clone(),
        run_mode: body.mode.clone().or(default_run_mode).unwrap_or_default(),
        discovered_tool_names: vec![],
        step_history: vec![],
        is_recovery: false,
    };

    let model_id = match body.model_id {
        Some(model_id) => model_id,
        None => {
            let brain_model = state.runtime.runtime_config.default_brain_model().await;
            if brain_model.trim().is_empty() || brain_model == "default" {
                state.runtime.runtime_config.default_generate_model().await
            } else {
                brain_model
            }
        }
    };
    let model_id = model_id.trim().to_owned();
    if model_id.is_empty() || model_id == "default" {
        return AppApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_brain_provider",
            "No default LLM model configured. Set brain_model or generate_model, or provide model_id explicitly.",
        )
        .into_response();
    }

    let is_bedrock_model = model_id.contains('.') && !model_id.contains('/');
    let brain = match state
        .runtime
        .provider_registry
        .resolve_generation_for_model(
            &run.project.tenant_id,
            &model_id,
            cairn_runtime::ProviderResolutionPurpose::Brain,
        )
        .await
    {
        Ok(Some(provider)) => provider,
        Ok(None) => {
            if is_bedrock_model {
                match &state.bedrock_provider {
                    Some(provider) => provider.clone(),
                    None => {
                        return AppApiError::new(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "no_bedrock_provider",
                            "Bedrock model requested but AWS credentials not configured.",
                        )
                        .into_response();
                    }
                }
            } else {
                match &state.brain_provider {
                    Some(provider) => provider.clone(),
                    None => {
                        return AppApiError::new(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "no_brain_provider",
                            "No LLM provider configured. Add one via POST /v1/providers/connections, or set CAIRN_BRAIN_URL / OPENROUTER_API_KEY / OLLAMA_HOST.",
                        )
                        .into_response()
                    }
                }
            }
        }
        Err(err) => return runtime_error_response(err),
    };

    let gather = StandardGatherPhase::builder(state.runtime.store.clone())
        .with_retrieval(state.retrieval.clone())
        .with_graph(state.graph.clone())
        .with_defaults(state.runtime.store.clone())
        .with_checkpoints(state.runtime.store.clone())
        .build();

    // ── SSE notification sink for notify_operator ───────────────────────────
    // Wraps the broadcast channel so notify_operator can push realtime events.
    struct SseSink {
        tx: tokio::sync::broadcast::Sender<cairn_api::sse::SseFrame>,
        seq: std::sync::Arc<std::sync::atomic::AtomicU64>,
        buf: std::sync::Arc<
            std::sync::RwLock<std::collections::VecDeque<(u64, cairn_api::sse::SseFrame)>>,
        >,
    }
    #[async_trait::async_trait]
    impl NotificationSink for SseSink {
        async fn emit(&self, channel: &str, severity: &str, message: &str) {
            let frame = cairn_api::sse::SseFrame {
                event: cairn_api::sse::SseEventName::OperatorNotification,
                data: serde_json::json!({
                    "channel":  channel,
                    "severity": severity,
                    "message":  message,
                }),
                id: None,
                // NotificationSink has no scope context; keep tenant-agnostic.
                // Filtering happens in the SSE handler.
                tenant_id: None,
            };
            let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut frame_with_id = frame.clone();
            frame_with_id.id = Some(seq.to_string());
            {
                let mut buf = self.buf.write().unwrap_or_else(|e| e.into_inner());
                if buf.len() >= 10_000 {
                    buf.pop_front();
                }
                buf.push_back((seq, frame_with_id));
            }
            let _ = self.tx.send(frame);
        }
    }
    let sse_sink: std::sync::Arc<dyn NotificationSink> = std::sync::Arc::new(SseSink {
        tx: state.runtime_sse_tx.clone(),
        seq: state.sse_seq.clone(),
        buf: state.sse_event_buffer.clone(),
    });
    let mailbox_svc: std::sync::Arc<dyn cairn_runtime::MailboxService> = std::sync::Arc::new(
        cairn_runtime::services::MailboxServiceImpl::new(state.runtime.store.clone()),
    );

    // ── Build BuiltinToolRegistry ────────────────────────────────────────────
    // Wire all ~30 built-in tools (RFC 018 prerequisite).
    // Prefer real memory tool implementations (wired at startup with live
    // RetrievalService + IngestPipeline).  Fall back to stubs otherwise.
    let registry = {
        // Concrete memory tools: use real impl when state.tool_registry is set,
        // otherwise fall back to stubs (schema-correct but no backing service).
        let (search_tool, store_tool, register_repo_tool): (
            std::sync::Arc<dyn cairn_tools::ToolHandler>,
            std::sync::Arc<dyn cairn_tools::ToolHandler>,
            std::sync::Arc<dyn cairn_tools::ToolHandler>,
        ) = if let Some(ref real) = state.tool_registry {
            let search: std::sync::Arc<dyn cairn_tools::ToolHandler> = real
                .get("memory_search")
                .unwrap_or_else(|| std::sync::Arc::new(MemorySearchTool::new()));
            let store: std::sync::Arc<dyn cairn_tools::ToolHandler> = real
                .get("memory_store")
                .unwrap_or_else(|| std::sync::Arc::new(MemoryStoreTool::new()));
            let register_repo: std::sync::Arc<dyn cairn_tools::ToolHandler> =
                real.get("cairn.registerRepo").unwrap_or_else(|| {
                    std::sync::Arc::new(crate::tool_impls::ConcreteRegisterRepoTool::new(
                        state.project_repo_access.clone(),
                        state.repo_clone_cache.clone(),
                    ))
                });
            (search, store, register_repo)
        } else {
            (
                std::sync::Arc::new(MemorySearchTool::new()),
                std::sync::Arc::new(MemoryStoreTool::new()),
                std::sync::Arc::new(crate::tool_impls::ConcreteRegisterRepoTool::new(
                    state.project_repo_access.clone(),
                    state.repo_clone_cache.clone(),
                )),
            )
        };

        // Shared services needed by tool constructors
        let store_ref = state.runtime.store.clone();
        let workspace_root = working_dir.clone();
        let task_svc: Arc<dyn cairn_runtime::tasks::TaskService> = state.runtime.tasks.clone();
        let approval_svc: Arc<dyn cairn_runtime::ApprovalService> =
            Arc::new(ApprovalServiceImpl::new(store_ref.clone()));

        // ── Observational tools ─────────────────────────────────────────────
        let web_fetch: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(WebFetchTool::default());
        let grep_search: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(GrepSearchTool);
        let file_read: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(FileReadTool::new(workspace_root.clone()));
        let glob_find: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(GlobFindTool);
        let json_extract: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(JsonExtractTool);
        let calculate: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(CalculateTool);
        let graph_query: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(GraphQueryTool::new(state.graph.clone()));
        let get_run: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(GetRunTool::new(store_ref.clone()));
        let get_task: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(GetTaskTool::new(store_ref.clone()));
        let get_approvals: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(GetApprovalsTool::new(store_ref.clone()));
        let list_runs: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(ListRunsTool::new(store_ref.clone()));
        let search_events: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(SearchEventsTool::new(store_ref.clone()));
        let wait_for_task: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(WaitForTaskTool::new(store_ref.clone()));

        // ── Internal tools ──────────────────────────────────────────────────
        let scratch_pad: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(ScratchPadTool::new());
        let file_write: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(FileWriteTool::new(workspace_root.clone()));
        let create_task: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(CreateTaskTool::new(task_svc.clone()));
        let cancel_task: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(CancelTaskTool::new(task_svc));
        let summarize_text: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(SummarizeTextTool::new(brain.clone(), model_id.clone()));

        // ── External tools ──────────────────────────────────────────────────
        let shell_exec: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(ShellExecTool);
        let http_request: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(HttpRequestTool);
        let git_operations: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(GitOperationsTool::new(workspace_root));
        let resolve_approval: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(ResolveApprovalTool::new(approval_svc));
        let schedule_task: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(ScheduleTaskTool::new(store_ref.clone()));
        let score_tool: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(cairn_tools::EvalScoreTool::new(store_ref));

        // GitHub tools (Deferred -- discovered via tool_search)
        let gh_list: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(cairn_tools::GhListIssuesTool);
        let gh_get: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(cairn_tools::GhGetIssueTool);
        let gh_comment: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(cairn_tools::GhCreateCommentTool);
        let gh_search: std::sync::Arc<dyn cairn_tools::ToolHandler> =
            std::sync::Arc::new(cairn_tools::GhSearchCodeTool);

        // Helper: register all tools in a registry builder.
        let register_all = |reg: BuiltinToolRegistry| -> BuiltinToolRegistry {
            reg // Core / Observational
                .register(search_tool.clone())
                .register(store_tool.clone())
                .register(register_repo_tool.clone())
                .register(web_fetch.clone())
                .register(grep_search.clone())
                .register(file_read.clone())
                .register(glob_find.clone())
                .register(json_extract.clone())
                .register(calculate.clone())
                .register(graph_query.clone())
                .register(get_run.clone())
                .register(get_task.clone())
                .register(get_approvals.clone())
                .register(list_runs.clone())
                .register(search_events.clone())
                .register(wait_for_task.clone())
                // Internal
                .register(scratch_pad.clone())
                .register(file_write.clone())
                .register(create_task.clone())
                .register(cancel_task.clone())
                .register(summarize_text.clone())
                // External
                .register(shell_exec.clone())
                .register(std::sync::Arc::new(NotifyOperatorTool::new(
                    Some(mailbox_svc.clone()),
                    sse_sink.clone(),
                )))
                .register(http_request.clone())
                .register(git_operations.clone())
                .register(resolve_approval.clone())
                .register(schedule_task.clone())
                .register(score_tool.clone())
                // GitHub (Deferred)
                .register(gh_list.clone())
                .register(gh_get.clone())
                .register(gh_comment.clone())
                .register(gh_search.clone())
        };

        // Build inner registry for ToolSearchTool (includes deferred GH tools).
        let inner = std::sync::Arc::new(register_all(BuiltinToolRegistry::new()));

        // Full registry with ToolSearchTool that can search the deferred tier.
        std::sync::Arc::new(
            register_all(BuiltinToolRegistry::new())
                .register(std::sync::Arc::new(ToolSearchTool::new(inner))),
        )
    };

    let decide = LlmDecidePhase::new(brain, model_id.clone()).with_tools(registry.clone());

    // Build loop config first so checkpoint policy is available for execute.
    let mut cfg = LoopConfig::default();
    if let Some(m) = body.max_iterations {
        cfg.max_iterations = m;
    }
    if let Some(t) = body.timeout_ms {
        cfg.timeout_ms = t;
    }

    // Build RuntimeExecutePhase from the shared runtime store.
    // All service impls share the same Arc<InMemoryStore> so writes from one
    // service are immediately visible to reads from another.
    let store = state.runtime.store.clone();
    let execute = RuntimeExecutePhase::builder()
        .tool_registry(registry)
        .run_service(state.runtime.runs.clone())
        .task_service(state.runtime.tasks.clone())
        .approval_service(Arc::new(ApprovalServiceImpl::new(store.clone())))
        .checkpoint_service(Arc::new(CheckpointServiceImpl::new(store.clone())))
        .mailbox_service(Arc::new(MailboxServiceImpl::new(store.clone())))
        .tool_invocation_service(Arc::new(ToolInvocationServiceImpl::new(store)))
        .decision_service(Arc::new(
            crate::telemetry_routes::UsageMeteredDecisionService::new(
                state.runtime.decision_service.clone(),
                state.runtime.store.clone(),
            ),
        ))
        .checkpoint_every_n_tool_calls(cfg.checkpoint_every_n_tool_calls)
        .tool_result_cache(state.tool_result_cache.clone())
        .build();

    let sse_emitter = std::sync::Arc::new(crate::sse_hooks::SseOrchestratorEmitter::new(
        state.runtime_sse_tx.clone(),
        state.sse_event_buffer.clone(),
        state.sse_seq.clone(),
    ));

    // Composite emitter: SSE events + ProviderCallCompleted trace recording.
    struct TracingEmitter {
        inner: std::sync::Arc<crate::sse_hooks::SseOrchestratorEmitter>,
        store: std::sync::Arc<cairn_store::InMemoryStore>,
        exporter: std::sync::Arc<cairn_runtime::telemetry::OtlpExporter>,
    }
    #[async_trait::async_trait]
    impl cairn_orchestrator::OrchestratorEventEmitter for TracingEmitter {
        async fn on_started(&self, ctx: &cairn_orchestrator::OrchestrationContext) {
            self.inner.on_started(ctx).await;
        }
        async fn on_gather_completed(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            g: &cairn_orchestrator::GatherOutput,
        ) {
            self.inner.on_gather_completed(ctx, g).await;
        }
        async fn on_decide_completed(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            d: &cairn_orchestrator::DecideOutput,
        ) {
            self.inner.on_decide_completed(ctx, d).await;
            // Emit ProviderCallCompleted so LLM traces are populated.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let call_id = format!("orch_{}_{}", ctx.run_id.as_str(), now);
            let event = cairn_domain::EventEnvelope::for_runtime_event(
                cairn_domain::EventId::new(format!("evt_trace_{call_id}")),
                cairn_domain::EventSource::Runtime,
                cairn_domain::RuntimeEvent::ProviderCallCompleted(
                    cairn_domain::events::ProviderCallCompleted {
                        project: ctx.project.clone(),
                        provider_call_id: cairn_domain::ProviderCallId::new(&call_id),
                        route_decision_id: cairn_domain::RouteDecisionId::new(format!(
                            "rd_{call_id}"
                        )),
                        route_attempt_id: cairn_domain::RouteAttemptId::new(format!(
                            "ra_{call_id}"
                        )),
                        provider_binding_id: cairn_domain::ProviderBindingId::new("brain"),
                        provider_connection_id: cairn_domain::ProviderConnectionId::new("brain"),
                        provider_model_id: cairn_domain::ProviderModelId::new(&d.model_id),
                        operation_kind: cairn_domain::providers::OperationKind::Generate,
                        status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                        latency_ms: Some(d.latency_ms),
                        input_tokens: d.input_tokens,
                        output_tokens: d.output_tokens,
                        cost_micros: Some(
                            ((d.input_tokens.unwrap_or(0) as u64).saturating_mul(500)
                                + (d.output_tokens.unwrap_or(0) as u64).saturating_mul(1500))
                                / 1_000,
                        ),
                        completed_at: now,
                        session_id: Some(ctx.session_id.clone()),
                        run_id: Some(ctx.run_id.clone()),
                        error_class: None,
                        raw_error_message: None,
                        retry_count: 0,
                        task_id: ctx
                            .task_id
                            .as_ref()
                            .map(|t| cairn_domain::TaskId::new(t.as_str())),
                        prompt_release_id: None,
                        fallback_position: 0,
                        started_at: now.saturating_sub(d.latency_ms),
                        finished_at: now,
                    },
                ),
            );
            let payload = event.payload.clone();
            if let Err(e) = self.store.append(&[event]).await {
                tracing::warn!("event store append failed (non-fatal): {e}");
            }
            let _ = self.exporter.export_event(&payload).await;

            // Insert into the LlmCallTrace read model so /v1/traces is populated.
            use cairn_store::projections::LlmCallTraceReadModel;
            let input_tokens = d.input_tokens.unwrap_or(0);
            let output_tokens = d.output_tokens.unwrap_or(0);
            // Approximate cost: $0.50/M input, $1.50/M output (generic estimate).
            // Multiply first to avoid integer truncation on small token counts.
            let cost_micros = ((input_tokens as u64).saturating_mul(500)
                + (output_tokens as u64).saturating_mul(1500))
                / 1_000;
            let trace = cairn_domain::LlmCallTrace {
                trace_id: call_id,
                model_id: d.model_id.clone(),
                prompt_tokens: input_tokens,
                completion_tokens: output_tokens,
                latency_ms: d.latency_ms,
                cost_micros,
                session_id: Some(ctx.session_id.clone()),
                run_id: Some(ctx.run_id.clone()),
                created_at_ms: now,
                is_error: false,
            };
            let _ = self.store.insert_trace(trace).await;
        }
        async fn on_tool_called(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            name: &str,
            args: Option<&serde_json::Value>,
        ) {
            self.inner.on_tool_called(ctx, name, args).await;
        }
        async fn on_tool_result(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            name: &str,
            ok: bool,
            out: Option<&serde_json::Value>,
            err: Option<&str>,
            duration_ms: u64,
        ) {
            self.inner
                .on_tool_result(ctx, name, ok, out, err, duration_ms)
                .await;
        }
        async fn on_step_completed(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            d: &cairn_orchestrator::DecideOutput,
            e: &cairn_orchestrator::ExecuteOutcome,
        ) {
            self.inner.on_step_completed(ctx, d, e).await;
        }
        async fn on_finished(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            t: &cairn_orchestrator::LoopTermination,
        ) {
            self.inner.on_finished(ctx, t).await;
        }
    }
    let emitter: std::sync::Arc<dyn cairn_orchestrator::OrchestratorEventEmitter> =
        std::sync::Arc::new(TracingEmitter {
            inner: sse_emitter,
            store: state.runtime.store.clone(),
            exporter: state.otlp_exporter.clone(),
        });

    match OrchestratorLoop::new(gather, decide, execute, cfg)
        .with_emitter(emitter)
        .run(ctx)
        .await
    {
        Ok(LoopTermination::Completed { summary }) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "termination": "completed", "summary": summary, "model_id": model_id,
            })),
        )
            .into_response(),
        Ok(LoopTermination::Failed { reason }) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "termination": "failed", "reason": reason,
            })),
        )
            .into_response(),
        Ok(LoopTermination::MaxIterationsReached) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "termination": "max_iterations_reached",
            })),
        )
            .into_response(),
        Ok(LoopTermination::TimedOut) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "termination": "timed_out",
            })),
        )
            .into_response(),
        Ok(LoopTermination::WaitingApproval { approval_id }) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "termination": "waiting_approval", "approval_id": approval_id.as_str(),
            })),
        )
            .into_response(),
        Ok(LoopTermination::WaitingSubagent { child_task_id }) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "termination": "waiting_subagent", "child_task_id": child_task_id.as_str(),
            })),
        )
            .into_response(),
        Ok(LoopTermination::PlanProposed { plan_markdown }) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "termination": "plan_proposed",
                "outcome": "plan_proposed",
                "plan_markdown": plan_markdown,
            })),
        )
            .into_response(),
        Err(e) => {
            // T6a-H9: log the full error details (for ops) but send a
            // sanitized stable message to the client. The full Display
            // may embed provider URLs, model names, partial LLM output,
            // credential fragments, etc. — none of which belong in a 5xx
            // body. User-caused errors (NotFound, InvalidTransition)
            // still surface a friendly code + short message.
            tracing::warn!(run_id = %run_id, error = %e, "orchestration failed");
            let (status, code, msg): (_, &'static str, String) = match &e {
                cairn_orchestrator::OrchestratorError::Runtime(
                    cairn_runtime::error::RuntimeError::NotFound { .. },
                ) => (StatusCode::NOT_FOUND, "not_found", e.to_string()),
                cairn_orchestrator::OrchestratorError::Runtime(
                    cairn_runtime::error::RuntimeError::InvalidTransition { .. },
                ) => (StatusCode::CONFLICT, "invalid_transition", e.to_string()),
                cairn_orchestrator::OrchestratorError::Gather(_) => (
                    StatusCode::BAD_GATEWAY,
                    "gather_error",
                    "upstream gather phase failed".to_owned(),
                ),
                cairn_orchestrator::OrchestratorError::Decide(_) => (
                    StatusCode::BAD_GATEWAY,
                    "decide_error",
                    "upstream decide phase failed".to_owned(),
                ),
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "orchestration_error",
                    "orchestration failed — see server logs".to_owned(),
                ),
            };
            AppApiError::new(status, code, msg).into_response()
        }
    }
}

/// Deprecated stub. Manual recovery used to drive cairn-side
/// `RecoveryServiceImpl::recover_interrupted_runs`, but recovery now lives
/// unconditionally in FlowFabric's background scanners
/// (`LeaseExpiryScanner`, `AttemptTimeoutScanner`,
/// `ExecutionDeadlineScanner`, `SuspensionTimeoutScanner`,
/// `DependencyReconciler`, `UnblockScanner`, etc. — 14 total). Calling this
/// endpoint no longer does anything beyond confirming the run exists.
///
/// Kept as a 202 stub so operator dashboards hitting `/v1/runs/:id/recover`
/// don't break. Scheduled for removal in v2.
pub(crate) async fn recover_run_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let run_id = RunId::new(id);
    match state.runtime.runs.get(&run_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response()
        }
        Err(err) => return runtime_error_response(err),
    }

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "status": "accepted",
            "note": "recovery is handled by FlowFabric background scanners \
                     (lease_expiry, attempt_timeout, execution_deadline, \
                     suspension_timeout, dependency_reconciler, unblock_scanner, \
                     and 8 others); this endpoint is a no-op kept for \
                     backwards-compatibility and will be removed in v2",
            "deprecated": true,
        })),
    )
        .into_response()
}

pub(crate) async fn list_tenant_costs_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Query(query): Query<TenantCostQuery>,
) -> impl IntoResponse {
    match SessionCostReadModel::list_by_tenant(
        state.runtime.store.as_ref(),
        tenant_scope.tenant_id(),
        query.since_ms.unwrap_or(0),
    )
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
        Err(err) => store_error_response(err),
    }
}

// ── Plan review handlers (RFC 018) ───────────────────────────────────────────

/// POST /v1/runs/:plan_run_id/approve
pub(crate) async fn approve_plan_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(plan_run_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    use cairn_domain::events::PlanApproved;
    use cairn_runtime::make_envelope;

    let run_id = RunId::new(&plan_run_id);

    // T6a-C2: tenant scope check.
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    let reviewer_comments = body
        .get("reviewer_comments")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // T6a-H7: use the authenticated principal as the actor rather than a
    // hardcoded "operator" literal. Pre-fix, every plan approval was
    // attributed to a fake user — breaking audit integrity in team
    // deployments where multiple operators review plans.
    let evt = make_envelope(cairn_domain::RuntimeEvent::PlanApproved(PlanApproved {
        project: run.project.clone(),
        plan_run_id: run_id,
        approved_by: cairn_domain::OperatorId::new(crate::handlers::admin::audit_actor_id(
            &principal,
        )),
        reviewer_comments,
        approved_at: now_ms,
    }));

    if let Err(e) = state.runtime.store.append(&[evt]).await {
        return AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            e.to_string(),
        )
        .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "plan_run_id": plan_run_id,
            "status": "approved",
            "next_step": "create_execute_run",
        })),
    )
        .into_response()
}

/// POST /v1/runs/:plan_run_id/reject
pub(crate) async fn reject_plan_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(plan_run_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    use cairn_domain::events::PlanRejected;
    use cairn_runtime::make_envelope;

    let run_id = RunId::new(&plan_run_id);

    // T6a-C2: tenant scope check.
    let run = match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &run_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(response) => return response,
    };

    let reason = body
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("rejected by operator")
        .to_owned();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // T6a-H7: audit with the real principal, not a hardcoded "operator".
    let evt = make_envelope(cairn_domain::RuntimeEvent::PlanRejected(PlanRejected {
        project: run.project.clone(),
        plan_run_id: run_id,
        rejected_by: cairn_domain::OperatorId::new(crate::handlers::admin::audit_actor_id(
            &principal,
        )),
        reason,
        rejected_at: now_ms,
    }));

    if let Err(e) = state.runtime.store.append(&[evt]).await {
        return AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            e.to_string(),
        )
        .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "plan_run_id": plan_run_id,
            "status": "rejected",
        })),
    )
        .into_response()
}

/// POST /v1/runs/:plan_run_id/revise
pub(crate) async fn revise_plan_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(plan_run_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    use cairn_domain::events::PlanRevisionRequested;
    use cairn_runtime::make_envelope;

    let original_run_id = RunId::new(&plan_run_id);

    // T6a-C2: tenant scope check.
    let original_run =
        match load_run_visible_to_tenant(state.as_ref(), &tenant_scope, &original_run_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                    .into_response();
            }
            Err(response) => return response,
        };

    let reviewer_comments = body
        .get("reviewer_comments")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    if reviewer_comments.is_empty() {
        return bad_request_response("reviewer_comments is required for revise");
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Create a new Plan-mode run for the revision.
    let new_run_id = RunId::new(format!("run_{now_ms}_rev"));
    let before = current_event_head(&state).await;
    match state
        .runtime
        .runs
        .start(
            &original_run.project,
            &original_run.session_id,
            new_run_id.clone(),
            Some(original_run_id.clone()),
        )
        .await
    {
        Ok(_) => {}
        Err(err) => return runtime_error_response(err),
    }

    // Emit PlanRevisionRequested event.
    let evt = make_envelope(cairn_domain::RuntimeEvent::PlanRevisionRequested(
        PlanRevisionRequested {
            project: original_run.project.clone(),
            original_plan_run_id: original_run_id,
            new_plan_run_id: new_run_id.clone(),
            reviewer_comments,
            requested_at: now_ms,
        },
    ));

    if let Err(e) = state.runtime.store.append(&[evt]).await {
        return AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            e.to_string(),
        )
        .into_response();
    }

    publish_runtime_frames_since(&state, before).await;

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "plan_run_id": plan_run_id,
            "new_plan_run_id": new_run_id.as_str(),
            "status": "revision_requested",
        })),
    )
        .into_response()
}

// ── Record checkpoint ────────────────────────────────────────────────────────

/// `POST /v1/runs/:id/checkpoint` -- record a checkpoint for a run.
///
/// Alias for `save_checkpoint_handler`; provides the `record_checkpoint_handler`
/// name expected by the preserved route catalog and audit tests.
#[allow(dead_code)]
pub(crate) async fn record_checkpoint_handler(
    state: State<Arc<AppState>>,
    path: Path<String>,
    body: Json<crate::SaveCheckpointRequest>,
) -> impl IntoResponse {
    crate::save_checkpoint_handler(state, path, body).await
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn req(
        tenant: &str,
        workspace: &str,
        project: &str,
        session: &str,
        run: &str,
    ) -> CreateRunRequest {
        CreateRunRequest {
            tenant_id: tenant.into(),
            workspace_id: workspace.into(),
            project_id: project.into(),
            session_id: session.into(),
            run_id: run.into(),
            parent_run_id: None,
            mode: None,
        }
    }

    #[test]
    fn validate_accepts_normal_ids() {
        assert!(req("t1", "w1", "p1", "s1", "r1").validate().is_ok());
    }

    /// SEC-002: embedded NUL bytes are FF delimiters under RFC-011; rejecting
    /// them at the HTTP boundary prevents tenant-scope collapse via id_map.
    #[test]
    fn validate_rejects_tenant_id_with_null_byte() {
        let r = req("tenant\0bad", "w1", "p1", "s1", "r1");
        let err = r.validate().unwrap_err();
        assert!(err.contains("tenant_id"));
        assert!(err.contains("control characters"));
    }

    #[test]
    fn validate_rejects_workspace_id_with_soh() {
        let r = req("t1", "ws\x01bad", "p1", "s1", "r1");
        let err = r.validate().unwrap_err();
        assert!(err.contains("workspace_id"));
        assert!(err.contains("control characters"));
    }

    #[test]
    fn validate_rejects_project_id_with_newline() {
        let r = req("t1", "w1", "proj\nbad", "s1", "r1");
        let err = r.validate().unwrap_err();
        assert!(err.contains("project_id"));
        assert!(err.contains("control characters"));
    }

    #[test]
    fn validate_rejects_empty_run_id() {
        let r = req("t1", "w1", "p1", "s1", "");
        let err = r.validate().unwrap_err();
        assert!(err.contains("run_id"));
        assert!(err.contains("required"));
    }

    #[test]
    fn validate_rejects_empty_session_id() {
        let r = req("t1", "w1", "p1", "", "r1");
        let err = r.validate().unwrap_err();
        assert!(err.contains("session_id"));
        assert!(err.contains("required"));
    }

    #[test]
    fn validate_rejects_oversized_tenant_id() {
        let r = req(
            &"x".repeat(crate::validate::MAX_ID_LEN + 1),
            "w1",
            "p1",
            "s1",
            "r1",
        );
        let err = r.validate().unwrap_err();
        assert!(err.contains("tenant_id"));
        assert!(err.contains("maximum length"));
    }

    /// parent_run_id is optional — absent or empty is ok, control-chars are not.
    #[test]
    fn validate_parent_run_id_optional_but_checked() {
        let mut r = req("t1", "w1", "p1", "s1", "r1");
        assert!(r.validate().is_ok());
        r.parent_run_id = Some("parent\x07id".into());
        assert!(r.validate().is_err());
    }
}
