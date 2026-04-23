//! Prompt asset, version, release, and rollout HTTP handlers.

use std::collections::HashMap;
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
    AuditOutcome, ProjectKey, PromptAssetId, PromptReleaseId, PromptTemplateVar, PromptVersionId,
    WorkspaceKey,
};
use cairn_runtime::{AuditService, PromptAssetService, PromptReleaseService, PromptVersionService};
use cairn_store::projections::{PromptReleaseReadModel, PromptVersionReadModel};
use cairn_store::{EntityRef, EventLog};

use crate::errors::{runtime_error_response, store_error_response, AppApiError};
use crate::extractors::{OptionalProjectScopedQuery, ReviewerRoleGuard};
use crate::state::{AppState, AppVersionContent};
use crate::{
    audit_actor_id, latest_eval_score_for_release, DEFAULT_PROJECT_ID, DEFAULT_TENANT_ID,
    DEFAULT_WORKSPACE_ID,
};
use cairn_domain::RuntimeEvent;

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct ReleaseCompareEntry {
    pub(crate) release_id: String,
    pub(crate) state: String,
    pub(crate) version_number: Option<u32>,
    pub(crate) content_preview: String,
    pub(crate) eval_score: Option<f64>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct CompareResponse {
    pub(crate) releases: Vec<ReleaseCompareEntry>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct TransitionRecord {
    pub(crate) from_state: String,
    pub(crate) to_state: String,
    pub(crate) actor: Option<String>,
    pub(crate) timestamp: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct PromptVersionDiffResponse {
    pub(crate) added_lines: Vec<String>,
    pub(crate) removed_lines: Vec<String>,
    pub(crate) unchanged_lines: Vec<String>,
    pub(crate) similarity_score: f64,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreatePromptAssetRequest {
    pub(crate) tenant_id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) prompt_asset_id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
}

impl CreatePromptAssetRequest {
    #[allow(dead_code)]
    pub(crate) fn workspace(&self) -> WorkspaceKey {
        WorkspaceKey::new(
            self.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
            self.workspace_id.as_deref().unwrap_or(DEFAULT_WORKSPACE_ID),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreatePromptVersionRequest {
    pub(crate) tenant_id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) project_id: Option<String>,
    /// Optional — server mints `pv_<uuid>` when absent or empty.
    #[serde(default)]
    pub(crate) prompt_version_id: Option<String>,
    pub(crate) content_hash: String,
    pub(crate) content: Option<String>,
    pub(crate) template_vars: Option<Vec<PromptTemplateVar>>,
}

impl CreatePromptVersionRequest {
    #[allow(dead_code)]
    pub(crate) fn workspace(&self) -> WorkspaceKey {
        WorkspaceKey::new(
            self.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
            self.workspace_id.as_deref().unwrap_or(DEFAULT_WORKSPACE_ID),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreatePromptReleaseRequest {
    pub(crate) tenant_id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) project_id: Option<String>,
    /// Optional — server mints `rel_<uuid>` when absent or empty.
    #[serde(default)]
    pub(crate) prompt_release_id: Option<String>,
    pub(crate) prompt_asset_id: String,
    pub(crate) prompt_version_id: String,
}

impl CreatePromptReleaseRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
            self.workspace_id.as_deref().unwrap_or(DEFAULT_WORKSPACE_ID),
            self.project_id.as_deref().unwrap_or(DEFAULT_PROJECT_ID),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct PromptReleaseTransitionRequest {
    pub(crate) to_state: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct PromptReleaseRollbackRequest {
    pub(crate) target_release_id: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct PromptReleaseCompareRequest {
    pub(crate) release_ids: Vec<String>,
    pub(crate) eval_dataset: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct StartRolloutRequest {
    pub(crate) percent: u8,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct PromptVersionDiffQuery {
    pub(crate) compare_to: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct RenderPromptVersionRequest {
    pub(crate) vars: HashMap<String, String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RenderPromptVersionResponse {
    pub(crate) content: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)] // T6a-H2: preserved for schema compat while the handler returns 501
pub(crate) struct RegisterModelRequest {
    pub(crate) model_id: String,
    pub(crate) operation_kinds: Vec<String>,
    pub(crate) context_window_tokens: Option<u32>,
    pub(crate) max_output_tokens: Option<u32>,
    pub(crate) supports_streaming: bool,
    pub(crate) cost_per_1k_input_tokens: Option<u64>,
    pub(crate) cost_per_1k_output_tokens: Option<u64>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

pub(crate) async fn list_prompt_assets_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    let workspace = WorkspaceKey::new(
        query.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
        query
            .workspace_id
            .as_deref()
            .unwrap_or(DEFAULT_WORKSPACE_ID),
    );
    match state
        .runtime
        .prompt_assets
        .list_by_workspace(
            &cairn_domain::TenantId::new(workspace.tenant_id.as_str()),
            &cairn_domain::WorkspaceId::new(workspace.workspace_id.as_str()),
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
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn create_prompt_asset_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreatePromptAssetRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .prompt_assets
        .create(
            &ProjectKey::new(
                body.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
                body.workspace_id.as_deref().unwrap_or(DEFAULT_WORKSPACE_ID),
                body.project_id.as_deref().unwrap_or(DEFAULT_PROJECT_ID),
            ),
            PromptAssetId::new(body.prompt_asset_id),
            body.name,
            body.kind,
        )
        .await
    {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_prompt_versions_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .prompt_versions
        .list_by_asset(&PromptAssetId::new(id), query.limit(), query.offset())
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

pub(crate) async fn create_prompt_version_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CreatePromptVersionRequest>,
) -> impl IntoResponse {
    let version_id_str = body
        .prompt_version_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("pv_{}", uuid::Uuid::new_v4().simple()));
    let content = body.content.clone().unwrap_or_default();
    let template_vars = body.template_vars.clone().unwrap_or_default();

    match state
        .runtime
        .prompt_versions
        .create(
            &ProjectKey::new(
                body.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
                body.workspace_id.as_deref().unwrap_or(DEFAULT_WORKSPACE_ID),
                body.project_id.as_deref().unwrap_or(DEFAULT_PROJECT_ID),
            ),
            PromptVersionId::new(version_id_str.clone()),
            PromptAssetId::new(id),
            body.content_hash,
        )
        .await
    {
        Ok(record) => {
            // Cache content and template vars (not carried in the event).
            state
                .version_content
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    version_id_str,
                    AppVersionContent {
                        content,
                        template_vars,
                    },
                );
            (StatusCode::CREATED, Json(record)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn render_prompt_version_handler(
    State(state): State<Arc<AppState>>,
    Path((id, version_id)): Path<(String, String)>,
    Json(body): Json<RenderPromptVersionRequest>,
) -> impl IntoResponse {
    let prompt_version_id = PromptVersionId::new(version_id.clone());
    let version_exists = match state.runtime.prompt_versions.get(&prompt_version_id).await {
        Ok(Some(record)) if record.prompt_asset_id != PromptAssetId::new(id.clone()) => {
            return AppApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                format!(
                    "prompt version {} not found for asset {}",
                    prompt_version_id, id
                ),
            )
            .into_response();
        }
        Ok(Some(_)) => true,
        Ok(None) => {
            return AppApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("prompt version not found: {}", prompt_version_id),
            )
            .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };
    let _ = version_exists;

    let cached = state
        .version_content
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&version_id)
        .cloned();
    let (content_template, template_vars) = match cached {
        Some(vc) => (vc.content, vc.template_vars),
        None => {
            return (
                StatusCode::OK,
                Json(RenderPromptVersionResponse {
                    content: String::new(),
                }),
            )
                .into_response();
        }
    };

    // Validate required vars and apply defaults.
    let mut rendered = content_template.clone();
    for var in &template_vars {
        let value = if let Some(v) = body.vars.get(&var.name) {
            v.clone()
        } else if let Some(ref default) = var.default_value {
            default.clone()
        } else if var.required {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "code": "validation_error",
                    "message": format!("required template variable '{}' not provided", var.name)
                })),
            )
                .into_response();
        } else {
            continue;
        };
        rendered = rendered.replace(&format!("{{{{{}}}}}", var.name), &value);
    }

    (
        StatusCode::OK,
        Json(RenderPromptVersionResponse { content: rendered }),
    )
        .into_response()
}

pub(crate) async fn list_prompt_template_vars_handler(
    State(state): State<Arc<AppState>>,
    Path((id, version_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let prompt_version_id = PromptVersionId::new(version_id.clone());
    match state.runtime.prompt_versions.get(&prompt_version_id).await {
        Ok(Some(record)) if record.prompt_asset_id == PromptAssetId::new(id) => {
            let vars = state
                .version_content
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&version_id)
                .map(|vc| vc.template_vars.clone())
                .unwrap_or_default();
            (StatusCode::OK, Json(vars)).into_response()
        }
        Ok(Some(_)) | Ok(None) => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("prompt version not found: {}", prompt_version_id),
        )
        .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_prompt_releases_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .prompt_releases
        .list_by_project(&query.project(), query.limit(), query.offset())
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

pub(crate) async fn create_prompt_release_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreatePromptReleaseRequest>,
) -> impl IntoResponse {
    let release_id = body
        .prompt_release_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("rel_{}", uuid::Uuid::new_v4().simple()));
    match state
        .runtime
        .prompt_releases
        .create(
            &body.project(),
            PromptReleaseId::new(release_id),
            PromptAssetId::new(body.prompt_asset_id),
            PromptVersionId::new(body.prompt_version_id),
        )
        .await
    {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn transition_prompt_release_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PromptReleaseTransitionRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .prompt_releases
        .transition(&PromptReleaseId::new(id), &body.to_state)
        .await
    {
        Ok(record) => (StatusCode::OK, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn activate_prompt_release_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    _role: ReviewerRoleGuard,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state
        .runtime
        .prompt_releases
        .activate(&PromptReleaseId::new(id))
        .await
    {
        Ok(record) => match state
            .runtime
            .audits
            .record(
                record.project.tenant_id.clone(),
                audit_actor_id(&principal),
                "activate_prompt_release".to_owned(),
                "prompt_release".to_owned(),
                record.prompt_release_id.to_string(),
                AuditOutcome::Success,
                serde_json::json!({
                    "prompt_asset_id": record.prompt_asset_id,
                    "state": record.state
                }),
            )
            .await
        {
            Ok(_) => (StatusCode::OK, Json(record)).into_response(),
            Err(err) => runtime_error_response(err),
        },
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn rollback_prompt_release_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PromptReleaseRollbackRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .prompt_releases
        .rollback(
            &PromptReleaseId::new(id),
            &PromptReleaseId::new(body.target_release_id),
        )
        .await
    {
        Ok(record) => (StatusCode::OK, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn request_prompt_release_approval_handler(
    State(state): State<Arc<AppState>>,
    Path(release_id): Path<String>,
) -> impl IntoResponse {
    let release_id = PromptReleaseId::new(release_id);
    match state
        .runtime
        .prompt_releases
        .request_approval(&release_id)
        .await
    {
        Ok(approval) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "approval_id": approval.approval_id.as_str(),
                "release_id": release_id.as_str(),
                "decision": approval.decision,
                "created_at": approval.created_at,
            })),
        )
            .into_response(),
        Err(crate::RuntimeError::NotFound { entity, id }) => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{entity} not found: {id}"),
        )
        .into_response(),
        Err(err) => AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            err.to_string(),
        )
        .into_response(),
    }
}

pub(crate) async fn register_provider_model_handler(
    State(_state): State<Arc<AppState>>,
    Path(_connection_id): Path<String>,
    Json(_body): Json<RegisterModelRequest>,
) -> impl IntoResponse {
    // T6a-H2: ProviderModelServiceImpl requires a ProviderModelReadModel
    // which InMemoryStore does not implement. Returning 200 + the
    // echoed body led operators to believe models had been registered.
    // Surface 501 Not Implemented so the UI can render a clear state
    // and an operator doesn't silently lose model config.
    AppApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "not_implemented",
        "provider model registration is not yet wired through ProviderModelServiceImpl",
    )
    .into_response()
}

pub(crate) async fn list_provider_models_handler(
    State(_state): State<Arc<AppState>>,
    Path(_connection_id): Path<String>,
) -> impl IntoResponse {
    // T6a-H2: same as register_provider_model_handler — return 501 so
    // callers don't mistake an empty array for "no models registered yet".
    AppApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "not_implemented",
        "provider model listing is not yet wired through ProviderModelServiceImpl",
    )
    .into_response()
}

pub(crate) async fn start_prompt_rollout_handler(
    State(state): State<Arc<AppState>>,
    Path(release_id): Path<String>,
    Json(body): Json<StartRolloutRequest>,
) -> impl IntoResponse {
    let release_id = PromptReleaseId::new(release_id);
    match state
        .runtime
        .prompt_releases
        .start_rollout(&release_id, body.percent)
        .await
    {
        Ok(record) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "release_id": record.prompt_release_id.as_str(),
                "state": record.state,
                "rollout_percent": record.rollout_percent,
            })),
        )
            .into_response(),
        Err(crate::RuntimeError::NotFound { entity, id }) => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{entity} not found: {id}"),
        )
        .into_response(),
        Err(err) => AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            err.to_string(),
        )
        .into_response(),
    }
}

/// `POST /v1/prompts/releases/:id/rollout` -- start a gradual rollout for a release.
///
/// Alias for `start_prompt_rollout_handler`; provides the `start_rollout_handler`
/// name expected by the preserved route catalog and SSE integration tests.
#[allow(dead_code)]
pub(crate) async fn start_rollout_handler(
    state: State<Arc<AppState>>,
    path: Path<String>,
    body: Json<StartRolloutRequest>,
) -> impl IntoResponse {
    start_prompt_rollout_handler(state, path, body).await
}

pub(crate) async fn compare_prompt_releases_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PromptReleaseCompareRequest>,
) -> impl IntoResponse {
    let _ = body.eval_dataset.as_deref();
    let store = state.runtime.store.as_ref();
    let mut releases = Vec::with_capacity(body.release_ids.len());

    for release_id_raw in body.release_ids {
        let release_id = PromptReleaseId::new(release_id_raw);
        let release = match PromptReleaseReadModel::get(store, &release_id).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                return AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    format!("prompt release not found: {}", release_id.as_str()),
                )
                .into_response();
            }
            Err(err) => return store_error_response(err),
        };

        let version = match PromptVersionReadModel::get(store, &release.prompt_version_id).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                return AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    format!(
                        "prompt version not found for release {}: {}",
                        release.prompt_release_id.as_str(),
                        release.prompt_version_id.as_str()
                    ),
                )
                .into_response();
            }
            Err(err) => return store_error_response(err),
        };

        let version_number = if version.version_number > 0 {
            version.version_number
        } else {
            match PromptVersionReadModel::list_by_asset(store, &release.prompt_asset_id, 1000, 0)
                .await
            {
                Ok(records) => records
                    .into_iter()
                    .enumerate()
                    .find_map(|(index, record)| {
                        (record.prompt_version_id == version.prompt_version_id)
                            .then_some((index + 1) as u32)
                    })
                    .unwrap_or(0),
                Err(err) => return store_error_response(err),
            }
        };

        releases.push(ReleaseCompareEntry {
            release_id: release.prompt_release_id.to_string(),
            state: release.state.clone(),
            version_number: Some(version_number),
            content_preview: version.content_hash.chars().take(200).collect(),
            eval_score: latest_eval_score_for_release(&state.evals, &release),
        });
    }

    (StatusCode::OK, Json(CompareResponse { releases })).into_response()
}

pub(crate) async fn prompt_release_history_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let events = match state
        .runtime
        .store
        .read_by_entity(
            &EntityRef::PromptRelease(PromptReleaseId::new(id)),
            None,
            1000,
        )
        .await
    {
        Ok(events) => events,
        Err(err) => return store_error_response(err),
    };

    let transitions = events
        .into_iter()
        .filter_map(|stored| match stored.envelope.payload {
            RuntimeEvent::PromptReleaseTransitioned(event) => Some(TransitionRecord {
                from_state: event.from_state.clone(),
                to_state: event.to_state.clone(),
                actor: None,
                timestamp: event.transitioned_at,
            }),
            _ => None,
        })
        .collect::<Vec<_>>();

    (StatusCode::OK, Json(transitions)).into_response()
}

pub(crate) async fn diff_prompt_versions_handler(
    State(state): State<Arc<AppState>>,
    Path((_asset_id, version_id)): Path<(String, String)>,
    Query(query): Query<PromptVersionDiffQuery>,
) -> impl IntoResponse {
    let cache = state
        .version_content
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let content_a = cache
        .get(&version_id)
        .map(|vc| vc.content.clone())
        .unwrap_or_default();
    let content_b = cache
        .get(&query.compare_to)
        .map(|vc| vc.content.clone())
        .unwrap_or_default();
    drop(cache);

    let lines_a: Vec<&str> = content_a.lines().collect();
    let lines_b: Vec<&str> = content_b.lines().collect();

    let set_a: std::collections::HashSet<&str> = lines_a.iter().copied().collect();
    let set_b: std::collections::HashSet<&str> = lines_b.iter().copied().collect();

    let added_lines: Vec<String> = lines_b
        .iter()
        .filter(|l| !set_a.contains(*l))
        .map(|l| l.to_string())
        .collect();
    let removed_lines: Vec<String> = lines_a
        .iter()
        .filter(|l| !set_b.contains(*l))
        .map(|l| l.to_string())
        .collect();
    let unchanged_lines: Vec<String> = lines_a
        .iter()
        .filter(|l| set_b.contains(*l))
        .map(|l| l.to_string())
        .collect();

    let total = lines_a.len() + lines_b.len();
    let similarity_score = if total == 0 {
        1.0_f64
    } else {
        (unchanged_lines.len() * 2) as f64 / total as f64
    };

    (
        StatusCode::OK,
        Json(PromptVersionDiffResponse {
            added_lines,
            removed_lines,
            unchanged_lines,
            similarity_score,
        }),
    )
        .into_response()
}
