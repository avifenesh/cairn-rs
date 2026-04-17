//! Session HTTP handlers and request/response DTOs.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::http::{ApiError, ListResponse};
use cairn_domain::{ProjectKey, SessionId};
use cairn_store::projections::{
    LlmCallTraceReadModel, RunCostReadModel, RunReadModel, RunRecord, SessionCostReadModel,
    SessionRecord, TaskReadModel,
};
use cairn_store::{EntityRef, EventLog, EventPosition, StoredEvent};
use utoipa::ToSchema;

use crate::errors::{
    bad_request_response, parse_session_state, runtime_error_response, store_error_response,
    AppApiError,
};
use crate::extractors::{HasProjectScope, ProjectJson, ProjectScope, TenantScope};
use crate::state::AppState;
use crate::{
    event_message, event_type_name, runtime_event_to_activity_entry, ActivityEntry, EventSummary,
    EventsPage, EventsPageQuery,
};
#[allow(unused_imports)]
use crate::{SessionListResponseDoc, SessionRecordDoc};

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct SessionDetailResponse {
    pub(crate) session: SessionRecord,
    pub(crate) runs: Vec<RunRecord>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct SessionActivity {
    pub(crate) session_id: String,
    pub(crate) entries: Vec<ActivityEntry>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct SessionCostResponse {
    #[serde(flatten)]
    pub(crate) summary: cairn_domain::providers::SessionCostRecord,
    pub(crate) run_breakdown: Vec<cairn_domain::providers::RunCostRecord>,
}

#[derive(Clone, Debug, serde::Deserialize, ToSchema)]
pub(crate) struct CreateSessionRequest {
    pub(crate) tenant_id: String,
    pub(crate) workspace_id: String,
    pub(crate) project_id: String,
    pub(crate) session_id: String,
}

impl CreateSessionRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

impl HasProjectScope for CreateSessionRequest {
    fn project(&self) -> ProjectKey {
        CreateSessionRequest::project(self)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SessionListQuery {
    pub(crate) tenant_id: String,
    pub(crate) workspace_id: String,
    pub(crate) project_id: String,
    pub(crate) status: Option<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
}

impl SessionListQuery {
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

impl HasProjectScope for SessionListQuery {
    fn project(&self) -> ProjectKey {
        SessionListQuery::project(self)
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/sessions",
    tag = "runtime",
    responses(
        (status = 200, description = "Sessions listed", body = SessionListResponseDoc),
        (status = 400, description = "Invalid request", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError)
    )
)]
pub(crate) async fn list_sessions_handler(
    State(state): State<Arc<AppState>>,
    project_scope: ProjectScope<SessionListQuery>,
) -> impl IntoResponse {
    let query = project_scope.into_inner();
    let status_filter = match query.status.as_deref().map(parse_session_state).transpose() {
        Ok(status_filter) => status_filter,
        Err(err) => return bad_request_response(err),
    };
    let limit = query.limit();

    match state
        .runtime
        .sessions
        .list(
            &SessionListQuery::project(&query),
            query.offset() + limit + 1,
            0,
        )
        .await
    {
        Ok(items) => {
            let mut items: Vec<SessionRecord> = items
                .into_iter()
                .filter(|session| {
                    status_filter.is_none_or(|status_filter| session.state == status_filter)
                })
                .skip(query.offset())
                .take(limit + 1)
                .collect();
            let has_more = items.len() > limit;
            items.truncate(limit);
            (StatusCode::OK, Json(ListResponse { items, has_more })).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn get_session_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.runtime.sessions.get(&SessionId::new(id)).await {
        Ok(Some(session)) if session.project.tenant_id == *tenant_scope.tenant_id() => {
            match RunReadModel::list_by_session(
                state.runtime.store.as_ref(),
                &session.session_id,
                200,
                0,
            )
            .await
            {
                Ok(runs) => (
                    StatusCode::OK,
                    Json(SessionDetailResponse { session, runs }),
                )
                    .into_response(),
                Err(err) => store_error_response(err),
            }
        }
        Ok(Some(_)) | Ok(None) => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "session not found")
                .into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn get_session_activity_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = SessionId::new(id.clone());

    match state.runtime.sessions.get(&session_id).await {
        Ok(Some(s)) if s.project.tenant_id == *tenant_scope.tenant_id() => {}
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "session not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    }

    let runs = match RunReadModel::list_by_session(
        state.runtime.store.as_ref(),
        &session_id,
        200,
        0,
    )
    .await
    {
        Ok(r) => r,
        Err(err) => return store_error_response(err),
    };

    let mut entries: Vec<ActivityEntry> = Vec::new();

    for run in &runs {
        // Read run-scoped events
        match state
            .runtime
            .store
            .read_by_entity(&EntityRef::Run(run.run_id.clone()), None, 200)
            .await
        {
            Ok(events) => {
                for stored in events {
                    if let Some(entry) =
                        runtime_event_to_activity_entry(&stored.envelope.payload, stored.stored_at)
                    {
                        entries.push(entry);
                    }
                }
            }
            Err(err) => return store_error_response(err),
        }

        // Read task-scoped events for each task in this run
        let tasks =
            match TaskReadModel::list_by_parent_run(state.runtime.store.as_ref(), &run.run_id, 200)
                .await
            {
                Ok(t) => t,
                Err(err) => return store_error_response(err),
            };

        for task in &tasks {
            match state
                .runtime
                .store
                .read_by_entity(&EntityRef::Task(task.task_id.clone()), None, 200)
                .await
            {
                Ok(events) => {
                    for stored in events {
                        if let Some(entry) = runtime_event_to_activity_entry(
                            &stored.envelope.payload,
                            stored.stored_at,
                        ) {
                            entries.push(entry);
                        }
                    }
                }
                Err(err) => return store_error_response(err),
            }
        }
    }

    entries.sort_by_key(|e| e.timestamp_ms);
    // Return last 100 entries
    let len = entries.len();
    if len > 100 {
        entries.drain(0..len - 100);
    }

    (
        StatusCode::OK,
        Json(SessionActivity {
            session_id: id,
            entries,
        }),
    )
        .into_response()
}

pub(crate) async fn get_session_active_runs_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = SessionId::new(id.clone());

    match state.runtime.sessions.get(&session_id).await {
        Ok(Some(s)) if s.project.tenant_id == *tenant_scope.tenant_id() => {}
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "session not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    }

    let runs = match RunReadModel::list_by_session(
        state.runtime.store.as_ref(),
        &session_id,
        200,
        0,
    )
    .await
    {
        Ok(r) => r,
        Err(err) => return store_error_response(err),
    };

    let active: Vec<RunRecord> = runs
        .into_iter()
        .filter(|r| !r.state.is_terminal())
        .collect();
    (
        StatusCode::OK,
        Json(ListResponse::<RunRecord> {
            items: active,
            has_more: false,
        }),
    )
        .into_response()
}

pub(crate) async fn get_session_cost_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = SessionId::new(id);
    match state.runtime.sessions.get(&session_id).await {
        Ok(Some(session)) if session.project.tenant_id == *tenant_scope.tenant_id() => {
            match SessionCostReadModel::get_session_cost(state.runtime.store.as_ref(), &session_id)
                .await
            {
                Ok(Some(record)) => {
                    match RunCostReadModel::list_by_session(
                        state.runtime.store.as_ref(),
                        &session_id,
                    )
                    .await
                    {
                        Ok(run_breakdown) => (
                            StatusCode::OK,
                            Json(SessionCostResponse {
                                summary: record,
                                run_breakdown,
                            }),
                        )
                            .into_response(),
                        Err(err) => store_error_response(err),
                    }
                }
                Ok(None) => {
                    AppApiError::new(StatusCode::NOT_FOUND, "not_found", "session cost not found")
                        .into_response()
                }
                Err(err) => store_error_response(err),
            }
        }
        Ok(Some(_)) | Ok(None) => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "session not found")
                .into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

/// `GET /v1/sessions/:id/llm-traces` — per-session LLM call trace history (GAP-010).
///
/// Returns up to 200 traces for the session, most-recent first.
/// Each trace records model, tokens, latency, and cost for one provider call.
pub(crate) async fn get_session_llm_traces_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = SessionId::new(id);

    // Verify the session exists and belongs to the requesting tenant.
    match state.runtime.sessions.get(&session_id).await {
        Ok(Some(s)) if s.project.tenant_id == *tenant_scope.tenant_id() => {}
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "session not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    }

    match LlmCallTraceReadModel::list_by_session(state.runtime.store.as_ref(), &session_id, 200)
        .await
    {
        Ok(traces) => (
            StatusCode::OK,
            Json(serde_json::json!({ "traces": traces })),
        )
            .into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn list_session_events_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
    Query(query): Query<EventsPageQuery>,
) -> impl IntoResponse {
    let session_id = SessionId::new(id);
    let session = match state.runtime.sessions.get(&session_id).await {
        Ok(Some(session)) if session.project.tenant_id == *tenant_scope.tenant_id() => session,
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "session not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };

    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let cursor = query.cursor.map(EventPosition);

    let fetched = match state
        .runtime
        .store
        .read_by_entity(
            &EntityRef::Session(session.session_id.clone()),
            cursor,
            limit + 1,
        )
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

    let events = page
        .into_iter()
        .map(|e| EventSummary {
            position: e.position.0,
            event_type: event_type_name(&e.envelope.payload).to_owned(),
            occurred_at_ms: e.stored_at,
            description: event_message(&e.envelope.payload),
        })
        .collect();

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

#[utoipa::path(
    post,
    path = "/v1/sessions",
    tag = "runtime",
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "Session created", body = SessionRecordDoc),
        (status = 400, description = "Invalid request", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 422, description = "Unprocessable entity", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError)
    )
)]
pub(crate) async fn create_session_handler(
    State(state): State<Arc<AppState>>,
    project_scope: ProjectJson<CreateSessionRequest>,
) -> impl IntoResponse {
    let body = project_scope.into_inner();
    match state
        .runtime
        .sessions
        .create(
            &CreateSessionRequest::project(&body),
            SessionId::new(body.session_id),
        )
        .await
    {
        Ok(session) => (StatusCode::CREATED, Json(session)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}
