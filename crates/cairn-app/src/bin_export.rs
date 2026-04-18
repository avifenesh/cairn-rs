//! Export / Import handlers for sessions and runs.

#[allow(unused_imports)]
use crate::*;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use cairn_domain::{ProjectKey, RunId};
use cairn_store::projections::{RunReadModel, SessionReadModel, TaskReadModel};
use serde::Deserialize;

// ── Export / Import handlers ──────────────────────────────────────────────────

/// Current export format version.  Increment when the shape changes
/// incompatibly so importers can detect version mismatches early.
pub(crate) const EXPORT_VERSION: &str = "1.0";

/// Build an ISO-8601 timestamp string from the current system time.
pub(crate) fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        1970 + secs / 31_557_600,
        ((secs % 31_557_600) / 2_629_800) + 1,
        ((secs % 2_629_800) / 86_400) + 1,
        (secs % 86_400) / 3_600,
        (secs % 3_600) / 60,
        secs % 60,
    )
}

/// `GET /v1/runs/:id/export` — export a run with all its tasks and events.
///
/// The response is a JSON document suitable for archiving or importing into
/// another cairn instance.  The `Content-Disposition` header prompts browsers
/// to download it as `run-<id>.json`.
pub(crate) async fn export_run_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let run_id = RunId::new(&id);

    // Fetch the run record.
    let run = match RunReadModel::get(state.runtime.store.as_ref(), &run_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(not_found(format!("run {id} not found"))),
        Err(e) => return Err(internal_error(e.to_string())),
    };

    // Fetch tasks.
    let tasks = match TaskReadModel::list_by_parent_run(state.runtime.store.as_ref(), &run_id, 2000)
        .await
    {
        Ok(t) => t,
        Err(e) => return Err(internal_error(e.to_string())),
    };

    // Fetch events (summaries — full payloads are not stored by default).
    let events = match state
        .runtime
        .store
        .read_by_entity(&cairn_store::EntityRef::Run(run_id), None, 2000)
        .await
    {
        Ok(evts) => evts
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "position":   e.position.0,
                    "stored_at":  e.stored_at,
                    "event_type": event_type_name(&e.envelope.payload),
                    "event_id":   e.envelope.event_id.as_str(),
                })
            })
            .collect::<Vec<_>>(),
        Err(e) => return Err(internal_error(e.to_string())),
    };

    let body = serde_json::json!({
        "version":     EXPORT_VERSION,
        "type":        "run_export",
        "exported_at": now_iso8601(),
        "data": {
            "run":    run,
            "tasks":  tasks,
            "events": events,
        }
    });

    let filename = format!("run-{id}.json");
    let content_disposition = format!("attachment; filename=\"{filename}\"");
    let mut resp = (StatusCode::OK, Json(body)).into_response();
    resp.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&content_disposition)
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}

/// `GET /v1/sessions/:id/export` — export a session with all runs, tasks, events.
pub(crate) async fn export_session_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let session_id = cairn_domain::SessionId::new(&id);

    // Fetch the session record.
    let session = match SessionReadModel::get(state.runtime.store.as_ref(), &session_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return Err(not_found(format!("session {id} not found"))),
        Err(e) => return Err(internal_error(e.to_string())),
    };

    // All runs in this session.
    let runs = match RunReadModel::list_by_session(
        state.runtime.store.as_ref(),
        &session_id,
        500,
        0,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return Err(internal_error(e.to_string())),
    };

    // All tasks for every run.
    let mut all_tasks: Vec<serde_json::Value> = Vec::new();
    for run in &runs {
        let rid = run.run_id.clone();
        if let Ok(tasks) =
            TaskReadModel::list_by_parent_run(state.runtime.store.as_ref(), &rid, 2000).await
        {
            for t in tasks {
                all_tasks.push(serde_json::to_value(t).unwrap_or(serde_json::Value::Null));
            }
        }
    }

    // Events for the session itself.
    let events = match state
        .runtime
        .store
        .read_by_entity(&cairn_store::EntityRef::Session(session_id), None, 2000)
        .await
    {
        Ok(evts) => evts
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "position":   e.position.0,
                    "stored_at":  e.stored_at,
                    "event_type": event_type_name(&e.envelope.payload),
                    "event_id":   e.envelope.event_id.as_str(),
                })
            })
            .collect::<Vec<_>>(),
        Err(e) => return Err(internal_error(e.to_string())),
    };

    let body = serde_json::json!({
        "version":     EXPORT_VERSION,
        "type":        "session_export",
        "exported_at": now_iso8601(),
        "data": {
            "session": session,
            "runs":    runs,
            "tasks":   all_tasks,
            "events":  events,
        }
    });

    let filename = format!("session-{id}.json");
    let content_disposition = format!("attachment; filename=\"{filename}\"");
    let mut resp = (StatusCode::OK, Json(body)).into_response();
    resp.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&content_disposition)
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}

/// Import body for `POST /v1/sessions/import`.
#[derive(Deserialize)]
pub(crate) struct ImportSessionBody {
    version: Option<String>,
    #[serde(rename = "type")]
    export_type: Option<String>,
    data: Option<ImportSessionData>,
}

#[derive(Deserialize)]
pub(crate) struct ImportSessionData {
    session: Option<serde_json::Value>,
}

/// `POST /v1/sessions/import` — re-create a session from a session export.
///
/// Only the session record itself is re-created; runs, tasks, and events are
/// **not** replayed (that would require a full event-log replay which is out
/// of scope for this endpoint).  Returns the newly created session record.
pub(crate) async fn import_session_handler(
    State(state): State<AppState>,
    Json(body): Json<ImportSessionBody>,
) -> impl axum::response::IntoResponse {
    // Validate version
    if let Some(ref v) = body.version {
        if v != EXPORT_VERSION {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ApiError {
                    code: "version_mismatch",
                    message: format!(
                        "export version {v} is not supported; expected {EXPORT_VERSION}"
                    ),
                }),
            ));
        }
    }

    // Validate type
    match body.export_type.as_deref() {
        Some("session_export") | None => {}
        Some(t) => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ApiError {
                    code: "wrong_export_type",
                    message: format!("expected 'session_export', got '{t}'"),
                }),
            ));
        }
    }

    let session_data = body.data.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                code: "missing_data",
                message: "import body must include a 'data' field".to_owned(),
            }),
        )
    })?;

    let session_json = session_data.session.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                code: "missing_session",
                message: "'data.session' is required".to_owned(),
            }),
        )
    })?;

    // Extract fields from the exported session record.
    let session_id_str = session_json
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("imported_session");

    let project_obj = session_json.get("project");
    let tenant_id = project_obj
        .and_then(|p| p.get("tenant_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let workspace_id = project_obj
        .and_then(|p| p.get("workspace_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let project_id = project_obj
        .and_then(|p| p.get("project_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let project = ProjectKey::new(tenant_id, workspace_id, project_id);
    let session_id = cairn_domain::SessionId::new(session_id_str);

    match state.runtime.sessions.create(&project, session_id).await {
        Ok(record) => Ok((StatusCode::CREATED, Json(serde_json::json!(record)))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                code: "create_failed",
                message: e.to_string(),
            }),
        )),
    }
}

// Approval handlers → bin_handlers.rs
