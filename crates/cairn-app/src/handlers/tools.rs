//! Tool invocation and checkpoint HTTP handlers.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::http::ListResponse;
use cairn_domain::tool_invocation::ToolInvocationTarget;
use cairn_domain::{
    CheckpointId, CheckpointStrategy, CheckpointStrategySet, ExecutionClass, ProjectKey, RunId,
    RuntimeEvent, SessionId, TaskId, ToolInvocationId,
};
use cairn_runtime::{CheckpointService, ToolInvocationService};
use cairn_store::projections::{
    CheckpointReadModel, CheckpointStrategyReadModel, RunReadModel, ToolInvocationReadModel,
};
use cairn_store::EventLog;

use crate::errors::{
    bad_request_response, now_ms, operator_event_envelope, runtime_error_response,
    store_error_response, validation_error_response, AppApiError,
};
use crate::extractors::TenantScope;
use crate::helpers::{build_run_replay_result, checkpoint_recorded_position};
use crate::state::AppState;
use crate::{
    cancel_plugin_invocation, current_event_head, parse_tool_invocation_state,
    publish_runtime_frames_since,
};

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct ToolInvocationListQuery {
    pub(crate) run_id: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
}

impl ToolInvocationListQuery {
    pub(crate) fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateToolInvocationRequest {
    pub(crate) tenant_id: String,
    pub(crate) workspace_id: String,
    pub(crate) project_id: String,
    pub(crate) invocation_id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) run_id: Option<String>,
    pub(crate) task_id: Option<String>,
    pub(crate) target: ToolInvocationTarget,
    pub(crate) execution_class: ExecutionClass,
}

impl CreateToolInvocationRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct CheckpointListQuery {
    pub(crate) run_id: Option<String>,
    pub(crate) limit: Option<usize>,
}

impl CheckpointListQuery {
    pub(crate) fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SaveCheckpointRequest {
    pub(crate) checkpoint_id: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SetCheckpointStrategyRequest {
    pub(crate) strategy_id: String,
    pub(crate) interval_ms: u64,
    pub(crate) max_checkpoints: u32,
    pub(crate) trigger_on_task_complete: bool,
}

// ── Handlers: Tool Invocations ──────────────────────────────────────────────

pub(crate) async fn list_tool_invocations_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ToolInvocationListQuery>,
) -> impl IntoResponse {
    let Some(run_id) = query.run_id.as_deref() else {
        return (
            StatusCode::OK,
            Json(
                ListResponse::<cairn_domain::tool_invocation::ToolInvocationRecord> {
                    items: Vec::new(),
                    has_more: false,
                },
            ),
        )
            .into_response();
    };

    let mut items = match ToolInvocationReadModel::list_by_run(
        state.runtime.store.as_ref(),
        &RunId::new(run_id),
        query.limit().saturating_add(query.offset()),
        0,
    )
    .await
    {
        Ok(items) => items,
        Err(err) => return store_error_response(err),
    };

    if let Some(state_filter) = query.state.as_deref() {
        let parsed = match parse_tool_invocation_state(state_filter) {
            Ok(state) => state,
            Err(message) => return bad_request_response(message),
        };
        items.retain(|item| item.state == parsed);
    }

    let items = items
        .into_iter()
        .skip(query.offset())
        .take(query.limit())
        .collect::<Vec<_>>();

    (
        StatusCode::OK,
        Json(ListResponse {
            items,
            has_more: false,
        }),
    )
        .into_response()
}

pub(crate) async fn get_tool_invocation_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match ToolInvocationReadModel::get(state.runtime.store.as_ref(), &ToolInvocationId::new(id))
        .await
    {
        Ok(Some(record)) => (StatusCode::OK, Json(record)).into_response(),
        Ok(None) => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "tool invocation not found",
        )
        .into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn get_tool_invocation_progress_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let invocation_id = ToolInvocationId::new(id);
    // Scan events for the latest ToolInvocationProgressUpdated for this invocation.
    let events = match state.runtime.store.read_stream(None, 10_000).await {
        Ok(e) => e,
        Err(err) => return store_error_response(err),
    };
    let latest = events.into_iter().rev().find_map(|stored| {
        if let RuntimeEvent::ToolInvocationProgressUpdated(p) = stored.envelope.payload {
            if p.invocation_id == invocation_id {
                return Some(p);
            }
        }
        None
    });
    match latest {
        Some(p) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "percent": p.progress_pct as f64 + 0.5,
                "message": p.message,
                "updated_at_ms": p.updated_at_ms,
            })),
        )
            .into_response(),
        None => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "tool invocation progress not found",
        )
        .into_response(),
    }
}

pub(crate) async fn create_tool_invocation_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateToolInvocationRequest>,
) -> impl IntoResponse {
    let before = current_event_head(&state).await;
    let project = body.project();
    let invocation_id = ToolInvocationId::new(body.invocation_id);
    match state
        .runtime
        .tool_invocations
        .record_start(
            &project,
            invocation_id.clone(),
            body.session_id.map(SessionId::new),
            body.run_id.map(RunId::new),
            body.task_id.map(TaskId::new),
            body.target,
            body.execution_class,
        )
        .await
    {
        Ok(()) => {
            publish_runtime_frames_since(&state, before).await;
            match ToolInvocationReadModel::get(state.runtime.store.as_ref(), &invocation_id).await {
                Ok(Some(record)) => (StatusCode::CREATED, Json(record)).into_response(),
                Ok(None) => {
                    tracing::error!("tool invocation not found after create: {invocation_id}");
                    AppApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal_error",
                        "tool invocation not found after create",
                    )
                    .into_response()
                }
                Err(err) => {
                    tracing::error!("tool invocation read after create failed: {err}");
                    AppApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal_error",
                        err.to_string(),
                    )
                    .into_response()
                }
            }
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn complete_tool_invocation_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let before = current_event_head(&state).await;
    let invocation_id = ToolInvocationId::new(id);
    let record =
        match ToolInvocationReadModel::get(state.runtime.store.as_ref(), &invocation_id).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                return AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "tool invocation not found",
                )
                .into_response();
            }
            Err(err) => return store_error_response(err),
        };

    let tool_name = match &record.target {
        ToolInvocationTarget::Builtin { tool_name } => tool_name.clone(),
        ToolInvocationTarget::Plugin { tool_name, .. } => tool_name.clone(),
    };

    match state
        .runtime
        .tool_invocations
        .record_completed(
            &record.project,
            invocation_id.clone(),
            record.task_id.clone(),
            tool_name,
            &[],
            None,
            None,
        )
        .await
    {
        Ok(()) => {
            publish_runtime_frames_since(&state, before).await;
            match ToolInvocationReadModel::get(state.runtime.store.as_ref(), &invocation_id).await {
                Ok(Some(updated)) => (StatusCode::OK, Json(updated)).into_response(),
                Ok(None) => {
                    tracing::error!("tool invocation not found after completion: {invocation_id}");
                    AppApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal_error",
                        "tool invocation not found after completion",
                    )
                    .into_response()
                }
                Err(err) => store_error_response(err),
            }
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn cancel_tool_invocation_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let invocation_id = ToolInvocationId::new(id);

    let record =
        match ToolInvocationReadModel::get(state.runtime.store.as_ref(), &invocation_id).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                return AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "tool invocation not found",
                )
                .into_response();
            }
            Err(err) => return store_error_response(err),
        };

    let tool_name = match &record.target {
        ToolInvocationTarget::Builtin { tool_name } => tool_name.clone(),
        ToolInvocationTarget::Plugin { tool_name, .. } => tool_name.clone(),
    };

    // Best-effort: send cancel RPC to the plugin if one is handling this invocation
    if let ToolInvocationTarget::Plugin { plugin_id, .. } = &record.target {
        if let Ok(mut host) = state.plugin_host.lock() {
            cancel_plugin_invocation(&mut host, plugin_id, invocation_id.as_str());
        }
    }

    let before = current_event_head(&state).await;
    match state
        .runtime
        .tool_invocations
        .record_failed(
            &record.project,
            invocation_id.clone(),
            record.task_id.clone(),
            tool_name,
            cairn_domain::tool_invocation::ToolInvocationOutcomeKind::Canceled,
            Some("cancelled_by_operator".to_owned()),
        )
        .await
    {
        Ok(()) => {
            publish_runtime_frames_since(&state, before).await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "cancelled": true })),
            )
                .into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

// ── Handlers: Checkpoints ───────────────────────────────────────────────────

pub(crate) async fn list_checkpoints_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CheckpointListQuery>,
) -> impl IntoResponse {
    let Some(run_id) = query.run_id.as_deref() else {
        return validation_error_response("run_id is required");
    };

    match state
        .runtime
        .checkpoints
        .list_by_run(&RunId::new(run_id), query.limit())
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

pub(crate) async fn get_checkpoint_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match CheckpointReadModel::get(state.runtime.store.as_ref(), &CheckpointId::new(id)).await {
        Ok(Some(record)) => (StatusCode::OK, Json(record)).into_response(),
        Ok(None) => AppApiError::new(StatusCode::NOT_FOUND, "not_found", "checkpoint not found")
            .into_response(),
        Err(err) => store_error_response(err),
    }
}

/// `POST /v1/checkpoints/:id/restore` -- restore a run to a specific checkpoint.
///
/// Alias for `POST /v1/runs/:run_id/replay-to-checkpoint?checkpoint_id=<id>`.
/// Looks up the checkpoint by ID to resolve the owning run, then replays the
/// run's event log up to the position where the checkpoint was recorded.
pub(crate) async fn restore_checkpoint_handler(
    State(state): State<Arc<AppState>>,
    Path(checkpoint_id_str): Path<String>,
) -> impl IntoResponse {
    let checkpoint_id = CheckpointId::new(&checkpoint_id_str);

    // Resolve the checkpoint -> run_id.
    let checkpoint = match CheckpointReadModel::get(state.runtime.store.as_ref(), &checkpoint_id)
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "checkpoint not found")
                .into_response();
        }
        Err(err) => return store_error_response(err),
    };

    // Find the event-log position at which the checkpoint was recorded.
    let position = match checkpoint_recorded_position(
        state.runtime.store.as_ref(),
        &checkpoint.checkpoint_id,
        &checkpoint.run_id,
    )
    .await
    {
        Ok(Some(p)) => p,
        Ok(None) => {
            return AppApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "checkpoint event not found in event log",
            )
            .into_response();
        }
        Err(err) => return store_error_response(err),
    };

    match build_run_replay_result(state.as_ref(), &checkpoint.run_id, None, Some(position.0)).await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn save_checkpoint_handler(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Json(body): Json<SaveCheckpointRequest>,
) -> impl IntoResponse {
    let run_id = RunId::new(run_id);
    let run = match RunReadModel::get(state.runtime.store.as_ref(), &run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response()
        }
        Err(err) => {
            tracing::error!("checkpoint save: failed to read run: {err}");
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response();
        }
    };

    let before = current_event_head(&state).await;
    match state
        .runtime
        .checkpoints
        .save(&run.project, &run_id, CheckpointId::new(body.checkpoint_id))
        .await
    {
        Ok(record) => {
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::CREATED, Json(record)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn get_checkpoint_strategy_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let run_id = RunId::new(run_id);
    let run = match state.runtime.runs.get(&run_id).await {
        Ok(Some(run)) if run.project.tenant_id == *tenant_scope.tenant_id() => run,
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };

    match CheckpointStrategyReadModel::get_by_run(state.runtime.store.as_ref(), &run.run_id).await {
        Ok(Some(strategy)) => (StatusCode::OK, Json(strategy)).into_response(),
        Ok(None) => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "checkpoint strategy not found",
        )
        .into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn set_checkpoint_strategy_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(run_id): Path<String>,
    Json(body): Json<SetCheckpointStrategyRequest>,
) -> impl IntoResponse {
    let run_id = RunId::new(run_id);
    let run = match state.runtime.runs.get(&run_id).await {
        Ok(Some(run)) if run.project.tenant_id == *tenant_scope.tenant_id() => run,
        Ok(Some(_)) | Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };

    let strategy = CheckpointStrategy {
        strategy_id: body.strategy_id.clone(),
        project: run.project.clone(),
        run_id: run.run_id.clone(),
        interval_ms: body.interval_ms,
        max_checkpoints: body.max_checkpoints,
        trigger_on_task_complete: body.trigger_on_task_complete,
    };

    // Emit the CheckpointStrategySet event with full fields so the projection
    // can restore them on query.
    let event =
        operator_event_envelope(RuntimeEvent::CheckpointStrategySet(CheckpointStrategySet {
            strategy_id: strategy.strategy_id.clone(),
            description: String::new(),
            set_at_ms: now_ms(),
            run_id: Some(run_id.clone()),
            interval_ms: body.interval_ms,
            max_checkpoints: body.max_checkpoints,
            trigger_on_task_complete: body.trigger_on_task_complete,
        }));

    let before = current_event_head(&state).await;
    match state.runtime.store.append(&[event]).await {
        Ok(_) => {
            publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(strategy)).into_response()
        }
        Err(err) => store_error_response(err),
    }
}
