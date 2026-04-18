//! HTTP route handlers for triggers and run templates — RFC 022.
//!
//! All routes are project-scoped:
//!   /v1/projects/:project/triggers/*
//!   /v1/projects/:project/run-templates/*

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use cairn_domain::decisions::RunMode;
use cairn_domain::ids::{EventId, OperatorId, RunTemplateId, TriggerId};
use cairn_domain::tenancy::ProjectKey;
use cairn_domain::{EventEnvelope, EventSource, RuntimeEvent};
use cairn_runtime::{
    RateLimitConfig, RunTemplate, SignalPattern, TemplateBudget, Trigger, TriggerCondition,
    TriggerError, TriggerEvent, TriggerState,
};
use cairn_store::EventLog;

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

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ListQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl ListQuery {
    fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(100)
    }

    fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
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

fn validate_project_segment(value: &str, field: &'static str) -> Result<(), String> {
    let is_valid = !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));

    if is_valid {
        Ok(())
    } else {
        Err(format!("{field} contains unsupported path characters"))
    }
}

fn project_key(project_id: &str) -> Result<ProjectKey, String> {
    if let Some((tenant_id, workspace_id, scoped_project_id)) =
        crate::parse_project_scope(project_id)
    {
        validate_project_segment(tenant_id, "tenant_id")?;
        validate_project_segment(workspace_id, "workspace_id")?;
        validate_project_segment(scoped_project_id, "project_id")?;
        return Ok(ProjectKey::new(tenant_id, workspace_id, scoped_project_id));
    }

    validate_project_segment(project_id, "project_id")?;
    Ok(ProjectKey::new(
        crate::DEFAULT_TENANT_ID,
        crate::DEFAULT_WORKSPACE_ID,
        project_id,
    ))
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

fn bad_request_response(message: impl Into<String>) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
}

fn not_found_response(entity: &str, id: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: format!("{entity} not found: {id}"),
        }),
    )
        .into_response()
}

async fn append_operator_runtime_event(
    state: &AppState,
    event: RuntimeEvent,
) -> Result<(), String> {
    let envelope = EventEnvelope::for_runtime_event(
        EventId::new(format!("evt_trigger_{}", uuid::Uuid::new_v4())),
        EventSource::Operator {
            operator_id: operator_id(),
        },
        event,
    );
    state
        .runtime
        .store
        .append(&[envelope])
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

// ── Trigger Handlers ────────────────────────────────────────────────────────

/// GET /v1/projects/:project/triggers
pub async fn list_triggers_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    let triggers = state.triggers.lock().unwrap();
    let project = match project_key(&project_id) {
        Ok(project) => project,
        Err(message) => return bad_request_response(message),
    };
    let mut list: Vec<&Trigger> = triggers.list_triggers_for_project(&project);
    list.sort_by_key(|r| r.id.clone());
    let list: Vec<&Trigger> = list
        .into_iter()
        .skip(query.offset())
        .take(query.limit())
        .collect();
    Json(serde_json::to_value(&list).expect("trigger/template list serialization")).into_response()
}

/// POST /v1/projects/:project/triggers
pub async fn create_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateTriggerRequest>,
) -> impl IntoResponse {
    let (event, persisted) = {
        let mut triggers = state.triggers.lock().unwrap();
        let now = now_ms();
        let project = match project_key(&project_id) {
            Ok(project) => project,
            Err(message) => return bad_request_response(message),
        };

        let trigger = Trigger {
            id: TriggerId::new(format!("trigger_{now}")),
            project,
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
            Ok(event) => {
                let persisted = crate::runtime_event_for_trigger_created(
                    triggers
                        .get_trigger(match &event {
                            TriggerEvent::TriggerCreated { trigger_id, .. } => trigger_id,
                            _ => unreachable!("create_trigger must emit TriggerCreated"),
                        })
                        .expect("created trigger must remain available"),
                );
                (event, persisted)
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response();
            }
        }
    };

    match append_operator_runtime_event(state.as_ref(), persisted).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error }),
        )
            .into_response(),
    }
}

/// GET /v1/projects/:project/triggers/:trigger_id
pub async fn get_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, trigger_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let triggers = state.triggers.lock().unwrap();
    let project = match project_key(&project_id) {
        Ok(project) => project,
        Err(message) => return bad_request_response(message),
    };
    match triggers.get_trigger(&TriggerId::new(&trigger_id)) {
        Some(trigger) if trigger.project == project => {
            Json(serde_json::to_value(trigger).expect("trigger serialization")).into_response()
        }
        _ => not_found_response("trigger", &trigger_id),
    }
}

/// DELETE /v1/projects/:project/triggers/:trigger_id
pub async fn delete_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, trigger_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let (event, persisted) = {
        let mut triggers = state.triggers.lock().unwrap();
        let project = match project_key(&project_id) {
            Ok(project) => project,
            Err(message) => return bad_request_response(message),
        };
        match triggers.get_trigger(&TriggerId::new(&trigger_id)) {
            Some(trigger) if trigger.project == project => {}
            _ => return not_found_response("trigger", &trigger_id),
        }

        match triggers.delete_trigger(&TriggerId::new(&trigger_id), operator_id()) {
            Ok(event) => {
                let persisted = crate::runtime_event_for_trigger_service_event(&project, &event)
                    .expect("delete trigger should persist");
                (event, persisted)
            }
            Err(e) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response();
            }
        }
    };

    match append_operator_runtime_event(state.as_ref(), persisted).await {
        Ok(()) => (
            StatusCode::OK,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error }),
        )
            .into_response(),
    }
}

/// POST /v1/projects/:project/triggers/:trigger_id/enable
pub async fn enable_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, trigger_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let (event, persisted) = {
        let mut triggers = state.triggers.lock().unwrap();
        let project = match project_key(&project_id) {
            Ok(project) => project,
            Err(message) => return bad_request_response(message),
        };
        match triggers.get_trigger(&TriggerId::new(&trigger_id)) {
            Some(trigger) if trigger.project == project => {}
            _ => return not_found_response("trigger", &trigger_id),
        }

        match triggers.enable_trigger(&TriggerId::new(&trigger_id), operator_id()) {
            Ok(event) => {
                let persisted = crate::runtime_event_for_trigger_service_event(&project, &event)
                    .expect("enable trigger should persist");
                (event, persisted)
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response();
            }
        }
    };

    match append_operator_runtime_event(state.as_ref(), persisted).await {
        Ok(()) => (
            StatusCode::OK,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error }),
        )
            .into_response(),
    }
}

/// POST /v1/projects/:project/triggers/:trigger_id/disable
pub async fn disable_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, trigger_id)): Path<(String, String)>,
    body: Option<Json<DisableRequest>>,
) -> impl IntoResponse {
    let reason = body.and_then(|Json(b)| b.reason);
    let (event, persisted) = {
        let mut triggers = state.triggers.lock().unwrap();
        let project = match project_key(&project_id) {
            Ok(project) => project,
            Err(message) => return bad_request_response(message),
        };
        match triggers.get_trigger(&TriggerId::new(&trigger_id)) {
            Some(trigger) if trigger.project == project => {}
            _ => return not_found_response("trigger", &trigger_id),
        }

        match triggers.disable_trigger(&TriggerId::new(&trigger_id), operator_id(), reason) {
            Ok(event) => {
                let persisted = crate::runtime_event_for_trigger_service_event(&project, &event)
                    .expect("disable trigger should persist");
                (event, persisted)
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response();
            }
        }
    };

    match append_operator_runtime_event(state.as_ref(), persisted).await {
        Ok(()) => (
            StatusCode::OK,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error }),
        )
            .into_response(),
    }
}

/// POST /v1/projects/:project/triggers/:trigger_id/resume
pub async fn resume_trigger_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, trigger_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let (event, persisted) = {
        let mut triggers = state.triggers.lock().unwrap();
        let project = match project_key(&project_id) {
            Ok(project) => project,
            Err(message) => return bad_request_response(message),
        };
        match triggers.get_trigger(&TriggerId::new(&trigger_id)) {
            Some(trigger) if trigger.project == project => {}
            _ => return not_found_response("trigger", &trigger_id),
        }

        match triggers.resume_trigger(&TriggerId::new(&trigger_id)) {
            Ok(event) => {
                let persisted = crate::runtime_event_for_trigger_service_event(&project, &event)
                    .expect("resume trigger should persist");
                (event, persisted)
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response();
            }
        }
    };

    match append_operator_runtime_event(state.as_ref(), persisted).await {
        Ok(()) => (
            StatusCode::OK,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error }),
        )
            .into_response(),
    }
}

// ── Run Template Handlers ───────────────────────────────────────────────────

/// GET /v1/projects/:project/run-templates
pub async fn list_run_templates_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    let triggers = state.triggers.lock().unwrap();
    let project = match project_key(&project_id) {
        Ok(project) => project,
        Err(message) => return bad_request_response(message),
    };
    let mut list: Vec<&RunTemplate> = triggers.list_templates_for_project(&project);
    list.sort_by_key(|r| r.id.clone());
    let list: Vec<&RunTemplate> = list
        .into_iter()
        .skip(query.offset())
        .take(query.limit())
        .collect();
    Json(serde_json::to_value(&list).expect("trigger/template list serialization")).into_response()
}

/// POST /v1/projects/:project/run-templates
pub async fn create_run_template_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateRunTemplateRequest>,
) -> impl IntoResponse {
    let (event, persisted) = {
        let mut triggers = state.triggers.lock().unwrap();
        let now = now_ms();
        let project = match project_key(&project_id) {
            Ok(project) => project,
            Err(message) => return bad_request_response(message),
        };

        let template = RunTemplate {
            id: RunTemplateId::new(format!("tmpl_{now}")),
            project,
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
        let persisted = crate::runtime_event_for_run_template_created(
            triggers
                .get_template(match &event {
                    TriggerEvent::RunTemplateCreated { template_id, .. } => template_id,
                    _ => unreachable!("create_template must emit RunTemplateCreated"),
                })
                .expect("created template must remain available"),
        );
        (event, persisted)
    };

    match append_operator_runtime_event(state.as_ref(), persisted).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error }),
        )
            .into_response(),
    }
}

/// GET /v1/projects/:project/run-templates/:template_id
pub async fn get_run_template_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, template_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let triggers = state.triggers.lock().unwrap();
    let project = match project_key(&project_id) {
        Ok(project) => project,
        Err(message) => return bad_request_response(message),
    };
    match triggers.get_template(&RunTemplateId::new(&template_id)) {
        Some(template) if template.project == project => {
            Json(serde_json::to_value(template).expect("template serialization")).into_response()
        }
        _ => not_found_response("run template", &template_id),
    }
}

/// DELETE /v1/projects/:project/run-templates/:template_id
/// Returns 409 if any trigger references it.
pub async fn delete_run_template_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, template_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let (event, persisted) = {
        let mut triggers = state.triggers.lock().unwrap();
        let project = match project_key(&project_id) {
            Ok(project) => project,
            Err(message) => return bad_request_response(message),
        };
        match triggers.get_template(&RunTemplateId::new(&template_id)) {
            Some(template) if template.project == project => {}
            _ => return not_found_response("run template", &template_id),
        }

        match triggers.delete_template(&RunTemplateId::new(&template_id), operator_id()) {
            Ok(event) => {
                let persisted = crate::runtime_event_for_trigger_service_event(&project, &event)
                    .expect("delete template should persist");
                (event, persisted)
            }
            Err(TriggerError::TemplateInUse { .. }) => {
                return (
                    StatusCode::CONFLICT,
                    Json(ErrorResponse {
                        error: "template is referenced by one or more triggers".to_string(),
                    }),
                )
                    .into_response();
            }
            Err(e) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response();
            }
        }
    };

    match append_operator_runtime_event(state.as_ref(), persisted).await {
        Ok(()) => (
            StatusCode::OK,
            Json(TriggerEventResponse {
                events: vec![event],
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use cairn_api::bootstrap::BootstrapConfig;
    use cairn_domain::decisions::RunMode;
    use cairn_domain::ids::{RunTemplateId, TriggerId};
    use cairn_runtime::{RunTemplate, SignalPattern, TemplateBudget, Trigger, TriggerState};
    use tower::ServiceExt;

    use super::{delete_trigger_handler, get_trigger_handler, operator_id, project_key, ListQuery};
    use crate::AppState;

    #[test]
    fn project_key_accepts_full_scope() {
        let scoped = project_key("tenant-a/workspace-a/project-a").unwrap();
        assert_eq!(scoped.tenant_id.as_str(), "tenant-a");
        assert_eq!(scoped.workspace_id.as_str(), "workspace-a");
        assert_eq!(scoped.project_id.as_str(), "project-a");
    }

    #[test]
    fn list_query_caps_page_size() {
        let query = ListQuery {
            limit: Some(500),
            offset: Some(11),
        };

        assert_eq!(query.limit(), 100);
        assert_eq!(query.offset(), 11);
    }

    fn test_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route(
                "/v1/projects/:project/triggers/:trigger_id",
                get(get_trigger_handler).delete(delete_trigger_handler),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn trigger_routes_do_not_cross_project_boundaries() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        let project_a = project_key("project-a").unwrap();
        let project_b = project_key("project-b").unwrap();
        let template_id = RunTemplateId::new("tmpl_shared");
        let trigger_id = TriggerId::new("trigger_shared");

        {
            let mut triggers = state.triggers.lock().unwrap();
            triggers.create_template(RunTemplate {
                id: template_id.clone(),
                project: project_a.clone(),
                name: "Template".to_string(),
                description: None,
                default_mode: RunMode::default(),
                system_prompt: "system".to_string(),
                initial_user_message: None,
                plugin_allowlist: None,
                tool_allowlist: None,
                budget: TemplateBudget::default(),
                sandbox_hint: None,
                required_fields: Vec::new(),
                created_by: operator_id(),
                created_at: 1,
                updated_at: 1,
            });
            triggers
                .create_trigger(Trigger {
                    id: trigger_id.clone(),
                    project: project_a.clone(),
                    name: "Trigger".to_string(),
                    description: None,
                    signal_pattern: SignalPattern {
                        signal_type: "signal.test".to_string(),
                        plugin_id: None,
                    },
                    conditions: Vec::new(),
                    run_template_id: template_id,
                    state: TriggerState::Enabled,
                    rate_limit: Default::default(),
                    max_chain_depth: 5,
                    created_by: operator_id(),
                    created_at: 1,
                    updated_at: 1,
                })
                .unwrap();
        }

        let router = test_router(state.clone());
        let get_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/projects/{}/triggers/{}",
                        project_b.project_id.as_str(),
                        trigger_id.as_str()
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::NOT_FOUND);

        let delete_response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/v1/projects/{}/triggers/{}",
                        project_b.project_id.as_str(),
                        trigger_id.as_str()
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NOT_FOUND);

        let triggers = state.triggers.lock().unwrap();
        let trigger = triggers
            .get_trigger(&trigger_id)
            .expect("trigger should remain");
        assert_eq!(trigger.project, project_a);
    }
}
