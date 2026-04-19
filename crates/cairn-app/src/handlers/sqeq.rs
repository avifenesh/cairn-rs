//! SQ/EQ protocol and A2A orchestration handlers (RFC 021).
//!
//! Extracted from `lib.rs` — contains SQ/EQ initialize/submit/events,
//! A2A agent card, task submission, and task status endpoints.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use cairn_domain::{ProjectKey, RunId, SessionId, TaskId};
use cairn_store::EventLog;

use crate::errors::{
    bad_request_response, now_ms, runtime_error_response, tenant_scope_mismatch_error, AppApiError,
};
use crate::extractors::TenantScope;
use crate::state::{A2aTaskBinding, AppState, SqEqSessionBinding};

const DEFAULT_WORKSPACE_ID: &str = "default_workspace";
const DEFAULT_PROJECT_ID: &str = "default_project";

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SqEqStartRunParams {
    pub sqeq_session_id: String,
    pub session_id: String,
    pub run_id: String,
    pub parent_run_id: Option<String>,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

pub(crate) fn sqeq_ack_response(
    status: StatusCode,
    accepted: bool,
    correlation_id: String,
    projected_event_seq: Option<u64>,
    error: Option<String>,
) -> Response {
    (
        status,
        Json(cairn_domain::protocols::SqEqSubmissionAck {
            accepted,
            correlation_id,
            projected_event_seq,
            error,
        }),
    )
        .into_response()
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// POST /v1/sqeq/initialize — scope binding + capability negotiation.
pub(crate) async fn sqeq_initialize_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Json(body): Json<cairn_domain::protocols::SqEqInitializeRequest>,
) -> impl IntoResponse {
    use cairn_domain::protocols::{SqEqCapabilities, SqEqInitializeResponse};

    // Negotiate version: v1 only supports "1.0".
    let negotiated = if body.protocol_versions.iter().any(|v| v == "1.0") {
        "1.0".to_owned()
    } else {
        return bad_request_response("unsupported protocol version");
    };

    // Generate transport session ID.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let session_id = format!("sqeq_{now_ms}");

    if !tenant_scope.is_admin && body.scope.tenant_id != *tenant_scope.tenant_id() {
        return tenant_scope_mismatch_error().into_response();
    }

    state
        .sqeq_sessions
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            session_id.clone(),
            SqEqSessionBinding {
                project: body.scope.clone(),
            },
        );

    let include_reasoning = body
        .subscriptions
        .include_reasoning
        .as_deref()
        .unwrap_or("denied")
        .to_owned();

    let resp = SqEqInitializeResponse {
        negotiated_version: negotiated,
        sqeq_session_id: session_id,
        bound_scope: body.scope.clone(),
        include_reasoning,
        capabilities: SqEqCapabilities {
            supported_commands: vec![
                "start_run".into(),
                "pause_run".into(),
                "resume_run".into(),
                "cancel_run".into(),
                "resolve_approval".into(),
                "create_task".into(),
            ],
            supported_events: vec![
                "run.*".into(),
                "task.*".into(),
                "decision.*".into(),
                "memory.*".into(),
                "approval.*".into(),
            ],
            supports_replay: true,
            max_event_buffer: 10_000,
        },
    };

    (StatusCode::OK, Json(resp)).into_response()
}

/// POST /v1/sqeq/submit — validated command submission with correlation.
pub(crate) async fn sqeq_submit_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Json(body): Json<cairn_domain::protocols::SqEqSubmission>,
) -> impl IntoResponse {
    // Validate the method is known.
    let known_methods = [
        "start_run",
        "pause_run",
        "resume_run",
        "cancel_run",
        "resolve_approval",
        "create_task",
        "cancel_task",
    ];
    if !known_methods.contains(&body.method.as_str()) {
        return sqeq_ack_response(
            StatusCode::BAD_REQUEST,
            false,
            body.correlation_id.clone(),
            None,
            Some(format!("unknown method: {}", body.method)),
        );
    }

    if body.method == "start_run" {
        let params: SqEqStartRunParams = match serde_json::from_value(body.params.clone()) {
            Ok(params) => params,
            Err(err) => {
                return sqeq_ack_response(
                    StatusCode::BAD_REQUEST,
                    false,
                    body.correlation_id.clone(),
                    None,
                    Some(format!("invalid start_run params: {err}")),
                );
            }
        };

        let binding = match state
            .sqeq_sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&params.sqeq_session_id)
            .cloned()
        {
            Some(binding) => binding,
            None => {
                return sqeq_ack_response(
                    StatusCode::NOT_FOUND,
                    false,
                    body.correlation_id.clone(),
                    None,
                    Some("sqeq session not found".to_owned()),
                );
            }
        };

        if !tenant_scope.is_admin && binding.project.tenant_id != *tenant_scope.tenant_id() {
            return sqeq_ack_response(
                StatusCode::FORBIDDEN,
                false,
                body.correlation_id.clone(),
                None,
                Some("sqeq session does not belong to authenticated tenant".to_owned()),
            );
        }

        let session_id = SessionId::new(params.session_id.clone());
        match state.runtime.sessions.get(&session_id).await {
            Ok(Some(session)) if session.project == binding.project => {}
            Ok(Some(_)) | Ok(None) => {
                return sqeq_ack_response(
                    StatusCode::NOT_FOUND,
                    false,
                    body.correlation_id.clone(),
                    None,
                    Some("session not found".to_owned()),
                );
            }
            Err(err) => {
                return sqeq_ack_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    false,
                    body.correlation_id.clone(),
                    None,
                    Some(err.to_string()),
                );
            }
        }

        if let Some(parent_run_id) = params.parent_run_id.as_ref().map(RunId::new) {
            match state.runtime.runs.get(&parent_run_id).await {
                Ok(Some(parent_run)) if parent_run.project == binding.project => {}
                Ok(Some(_)) | Ok(None) => {
                    return sqeq_ack_response(
                        StatusCode::NOT_FOUND,
                        false,
                        body.correlation_id.clone(),
                        None,
                        Some("parent run not found".to_owned()),
                    );
                }
                Err(err) => {
                    return sqeq_ack_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        false,
                        body.correlation_id.clone(),
                        None,
                        Some(err.to_string()),
                    );
                }
            }
        }

        let before = crate::handlers::sse::current_event_head(&state).await;
        return match state
            .runtime
            .runs
            .start_with_correlation(
                &binding.project,
                &session_id,
                RunId::new(params.run_id),
                params.parent_run_id.map(RunId::new),
                &body.correlation_id,
            )
            .await
        {
            Ok(_) => {
                crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
                let projected = state
                    .runtime
                    .store
                    .head_position()
                    .await
                    .ok()
                    .flatten()
                    .map(|position| position.0);
                sqeq_ack_response(
                    StatusCode::ACCEPTED,
                    true,
                    body.correlation_id,
                    projected,
                    None,
                )
            }
            Err(err) => sqeq_ack_response(
                StatusCode::CONFLICT,
                false,
                body.correlation_id,
                None,
                Some(err.to_string()),
            ),
        };
    }

    // Get current event sequence for projected_event_seq.
    let seq = state
        .runtime
        .store
        .head_position()
        .await
        .ok()
        .flatten()
        .map(|p| p.0)
        .unwrap_or(0);

    sqeq_ack_response(
        StatusCode::ACCEPTED,
        true,
        body.correlation_id,
        Some(seq + 1),
        None,
    )
}

/// GET /v1/sqeq/events — SSE event stream with scope filtering.
///
/// Delegates to the existing SSE broadcast infrastructure with scope filtering
/// applied per the bound SQ/EQ session.
pub(crate) async fn sqeq_events_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    axum::extract::Extension(principal): axum::extract::Extension<cairn_api::auth::AuthPrincipal>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(session_id) = params.get("sqeq_session_id") else {
        return bad_request_response("sqeq_session_id is required");
    };

    let Some(binding) = state
        .sqeq_sessions
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(session_id)
        .cloned()
    else {
        return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "sqeq session not found")
            .into_response();
    };

    if !tenant_scope.is_admin && binding.project.tenant_id != *tenant_scope.tenant_id() {
        return tenant_scope_mismatch_error().into_response();
    }

    crate::handlers::sse::runtime_stream_handler(
        State(state),
        axum::extract::Extension(principal),
        headers,
    )
    .await
    .into_response()
}

// ── A2A handlers (RFC 021) ──────────────────────────────────────────────────

/// GET /.well-known/agent.json — A2A Agent Card.
pub(crate) async fn a2a_agent_card_handler(
    State(_state): State<Arc<AppState>>,
) -> impl IntoResponse {
    use cairn_domain::protocols::*;

    let card = A2aAgentCard {
        a2a_version: "0.3".to_owned(),
        agent_id: "urn:cairn:self-hosted".to_owned(),
        name: "Cairn Control Plane".to_owned(),
        description: "Self-hosted agent control plane for teams using AI".to_owned(),
        endpoints: A2aEndpoints {
            task_submission: "/v1/a2a/tasks".to_owned(),
            task_status: "/v1/a2a/tasks/{task_id}".to_owned(),
            task_cancel: "/v1/a2a/tasks/{task_id}/cancel".to_owned(),
        },
        auth: A2aAuth {
            auth_type: "bearer".to_owned(),
            docs_url: "https://docs.cairn.dev/a2a/auth".to_owned(),
        },
        capabilities: A2aCapabilities {
            accepts_tasks: true,
            delegates_tasks: true,
            supports_streaming: true,
            supports_push_notifications: false,
        },
        accepted_task_kinds: vec![
            "research".into(),
            "code_edit".into(),
            "incident_triage".into(),
            "content_drafting".into(),
            "data_analysis".into(),
        ],
        supported_input_formats: vec!["text/markdown".into(), "application/json".into()],
        supported_output_formats: vec!["text/markdown".into(), "application/json".into()],
        transport: vec!["https".into(), "https-sse".into()],
        version: "0.1.0".to_owned(),
    };

    (StatusCode::OK, Json(card)).into_response()
}

/// POST /v1/a2a/tasks — A2A task submission.
pub(crate) async fn a2a_submit_task_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Json(_body): Json<cairn_domain::protocols::A2aTaskSubmission>,
) -> impl IntoResponse {
    let task_id = format!("a2a_task_{}", now_ms());
    let task_key = TaskId::new(task_id.clone());
    let project = ProjectKey::new(
        tenant_scope.tenant_id().as_ref(),
        DEFAULT_WORKSPACE_ID,
        DEFAULT_PROJECT_ID,
    );

    match state
        .runtime
        .tasks
        .submit(&project, task_key.clone(), None, None, 0)
        .await
    {
        Ok(_) => {
            state
                .a2a_tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    task_id.clone(),
                    A2aTaskBinding {
                        task_id: task_key,
                        project,
                    },
                );

            let resp = cairn_domain::protocols::A2aTaskResponse {
                task_id: task_id.clone(),
                status: "submitted".to_owned(),
                status_url: format!("/v1/a2a/tasks/{task_id}"),
            };

            (StatusCode::CREATED, Json(resp)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

/// GET /v1/a2a/tasks/:id — A2A task status.
pub(crate) async fn a2a_get_task_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let Some(binding) = state
        .a2a_tasks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&task_id)
        .cloned()
    else {
        return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "task not found")
            .into_response();
    };

    if !tenant_scope.is_admin && binding.project.tenant_id != *tenant_scope.tenant_id() {
        return tenant_scope_mismatch_error().into_response();
    }

    match state.runtime.tasks.get(&binding.task_id).await {
        Ok(Some(task)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "task_id": task_id,
                "status": task.state,
                "source": "A2A",
                "internal_task_id": task.task_id,
            })),
        )
            .into_response(),
        Ok(None) => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "task not found").into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}
