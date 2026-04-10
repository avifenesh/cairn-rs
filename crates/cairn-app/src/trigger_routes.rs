//! HTTP route handlers for triggers and run templates — RFC 022.
//!
//! All routes are project-scoped:
//!   /v1/projects/:project/triggers/*
//!   /v1/projects/:project/run-templates/*

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use cairn_domain::decisions::RunMode;
use cairn_domain::ids::{OperatorId, RunTemplateId, TriggerId};
use cairn_domain::tenancy::ProjectKey;
use cairn_runtime::{
    RateLimitConfig, RunTemplate, SignalPattern, TemplateBudget, Trigger, TriggerCondition,
    TriggerError, TriggerEvent, TriggerState,
};

use crate::AppState;

// ── Request DTOs ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateTriggerRequest {
    pub name: String,
    pub description: Option<String>,
    pub signal_type: String,
    pub plugin_id: Option<String>,
    pub conditions: Vec<TriggerCondition>,
    pub run_template_id: String,
    #[serde(default = "default_max_chain_depth")]
    pub max_chain_depth: u8,
    pub rate_limit: Option<RateLimitConfig>,
}

fn default_max_chain_depth() -> u8 {
    5
}

#[derive(Deserialize)]
pub struct DisableRequest {
    pub reason: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateRunTemplateRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub default_mode: RunMode,
    pub system_prompt: String,
    pub initial_user_message: Option<String>,
    pub plugin_allowlist: Option<Vec<String>>,
    pub tool_allowlist: Option<Vec<String>>,
    #[serde(default)]
    pub budget: TemplateBudget,
    pub sandbox_hint: Option<String>,
    #[serde(default)]
    pub required_fields: Vec<String>,
}

// ── Response DTOs ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct TriggerEventResponse {
    events: Vec<TriggerEvent>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn project_key(project_id: &str) -> ProjectKey {
    ProjectKey::new("default", "default", project_id)
}

fn operator_id() -> OperatorId {
    OperatorId::new("operator")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Trigger Handlers ────────────────────────────────────────────────────────

/// GET /v1/projects/:project/triggers
pub async fn list_triggers_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    let triggers = state.triggers.lock().unwrap();
    let project = project_key(&project_id);
    let list: Vec<&Trigger> = triggers.list_triggers_for_project(&project);
    Json(serde_json::to_value(&list).expect("trigger/template list serialization"))
}

/// POST /v1/projects/:project/triggers
pub async fn create_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateTriggerRequest>,
) -> impl IntoResponse {
    let mut triggers = state.triggers.lock().unwrap();
    let now = now_ms();

    let trigger = Trigger {
        id: TriggerId::new(format!("trigger_{now}")),
        project: project_key(&project_id),
        name: body.name,
        description: body.description,
        signal_pattern: SignalPattern {
            signal_type: body.signal_type,
            plugin_id: body.plugin_id,
        },
        conditions: body.conditions,
        run_template_id: RunTemplateId::new(body.run_template_id),
        state: TriggerState::Enabled,
        rate_limit: body.rate_limit.unwrap_or_default(),
        max_chain_depth: body.max_chain_depth,
        created_by: operator_id(),
        created_at: now,
        updated_at: now,
    };

    match triggers.create_trigger(trigger) {
        Ok(event) => (
            StatusCode::CREATED,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// GET /v1/projects/:project/triggers/:trigger_id
pub async fn get_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, trigger_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let triggers = state.triggers.lock().unwrap();
    match triggers.get_trigger(&TriggerId::new(&trigger_id)) {
        Some(trigger) => {
            Json(serde_json::to_value(trigger).expect("trigger serialization")).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("trigger not found: {trigger_id}"),
            }),
        )
            .into_response(),
    }
}

/// DELETE /v1/projects/:project/triggers/:trigger_id
pub async fn delete_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, trigger_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let mut triggers = state.triggers.lock().unwrap();
    match triggers.delete_trigger(&TriggerId::new(&trigger_id), operator_id()) {
        Ok(event) => (
            StatusCode::OK,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// POST /v1/projects/:project/triggers/:trigger_id/enable
pub async fn enable_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, trigger_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let mut triggers = state.triggers.lock().unwrap();
    match triggers.enable_trigger(&TriggerId::new(&trigger_id), operator_id()) {
        Ok(event) => (
            StatusCode::OK,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// POST /v1/projects/:project/triggers/:trigger_id/disable
pub async fn disable_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, trigger_id)): Path<(String, String)>,
    body: Option<Json<DisableRequest>>,
) -> impl IntoResponse {
    let mut triggers = state.triggers.lock().unwrap();
    let reason = body.and_then(|Json(b)| b.reason);
    match triggers.disable_trigger(&TriggerId::new(&trigger_id), operator_id(), reason) {
        Ok(event) => (
            StatusCode::OK,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// POST /v1/projects/:project/triggers/:trigger_id/resume
pub async fn resume_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, trigger_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let mut triggers = state.triggers.lock().unwrap();
    match triggers.resume_trigger(&TriggerId::new(&trigger_id)) {
        Ok(event) => (
            StatusCode::OK,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

// ── Run Template Handlers ───────────────────────────────────────────────────

/// GET /v1/projects/:project/run-templates
pub async fn list_run_templates_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    let triggers = state.triggers.lock().unwrap();
    let project = project_key(&project_id);
    let list: Vec<&RunTemplate> = triggers.list_templates_for_project(&project);
    Json(serde_json::to_value(&list).expect("trigger/template list serialization"))
}

/// POST /v1/projects/:project/run-templates
pub async fn create_run_template_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateRunTemplateRequest>,
) -> impl IntoResponse {
    let mut triggers = state.triggers.lock().unwrap();
    let now = now_ms();

    let template = RunTemplate {
        id: RunTemplateId::new(format!("tmpl_{now}")),
        project: project_key(&project_id),
        name: body.name,
        description: body.description,
        default_mode: body.default_mode,
        system_prompt: body.system_prompt,
        initial_user_message: body.initial_user_message,
        plugin_allowlist: body.plugin_allowlist,
        tool_allowlist: body.tool_allowlist,
        budget: body.budget,
        sandbox_hint: body.sandbox_hint,
        required_fields: body.required_fields,
        created_by: operator_id(),
        created_at: now,
        updated_at: now,
    };

    let event = triggers.create_template(template);
    (
        StatusCode::CREATED,
        Json(TriggerEventResponse {
            events: vec![event],
        }),
    )
}

/// GET /v1/projects/:project/run-templates/:template_id
pub async fn get_run_template_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, template_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let triggers = state.triggers.lock().unwrap();
    match triggers.get_template(&RunTemplateId::new(&template_id)) {
        Some(template) => {
            Json(serde_json::to_value(template).expect("template serialization")).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run template not found: {template_id}"),
            }),
        )
            .into_response(),
    }
}

/// DELETE /v1/projects/:project/run-templates/:template_id
/// Returns 409 if any trigger references it.
pub async fn delete_run_template_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, template_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let mut triggers = state.triggers.lock().unwrap();
    match triggers.delete_template(&RunTemplateId::new(&template_id), operator_id()) {
        Ok(event) => (
            StatusCode::OK,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(TriggerError::TemplateInUse { .. }) => (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "template is referenced by one or more triggers".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
