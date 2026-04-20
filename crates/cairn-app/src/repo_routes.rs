//! HTTP route handlers for project-scoped repo access — RFC 016.
//!
//! Routes:
//!   GET    /v1/projects/:project/repos
//!   POST   /v1/projects/:project/repos
//!   GET    /v1/projects/:project/repos/:owner/:repo
//!   DELETE /v1/projects/:project/repos/:owner/:repo

use std::sync::Arc;

use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use cairn_api::auth::AuthPrincipal;
use cairn_domain::{ActorRef, OperatorId, ProjectKey, RepoAccessContext};
use cairn_workspace::{RepoId, RepoStoreError};
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepoAllowlistEntryResponse {
    pub repo_id: String,
    pub clone_status: String,
    pub added_by: Option<String>,
    pub added_at: Option<u64>,
    pub last_used_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepoAllowlistListResponse {
    pub project: String,
    pub repos: Vec<RepoAllowlistEntryResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepoDetailResponse {
    pub project: String,
    pub repo_id: String,
    pub allowlisted: bool,
    pub clone_status: String,
    pub added_by: Option<String>,
    pub added_at: Option<u64>,
    pub last_used_at: Option<u64>,
    pub recent_sandbox_usage: Vec<String>,
    pub recent_register_repo_decisions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepoMutationResponse {
    pub project: String,
    pub repo_id: String,
    pub allowlisted: bool,
    pub clone_status: String,
    pub clone_created: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ErrorResponse {
    error: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AddRepoRequest {
    pub repo_id: String,
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

fn operator_id_from_principal(principal: &cairn_api::auth::AuthPrincipal) -> OperatorId {
    // T6b-C5: derive from the authenticated principal so repo mutations
    // land with the real actor in the event log.
    OperatorId::new(crate::handlers::admin::audit_actor_id(principal))
}

use crate::extractors::enforce_project_tenant;

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

fn project_key_from_path(project: &str) -> Result<ProjectKey, String> {
    if let Some((tenant_id, workspace_id, project_id)) = crate::parse_project_scope(project) {
        validate_project_segment(tenant_id, "tenant_id")?;
        validate_project_segment(workspace_id, "workspace_id")?;
        validate_project_segment(project_id, "project_id")?;
        return Ok(ProjectKey::new(tenant_id, workspace_id, project_id));
    }

    validate_project_segment(project, "project_id")?;
    Ok(ProjectKey::new(
        crate::DEFAULT_TENANT_ID,
        crate::DEFAULT_WORKSPACE_ID,
        project,
    ))
}

fn repo_access_context(project: &str) -> Result<RepoAccessContext, String> {
    Ok(RepoAccessContext {
        project: project_key_from_path(project)?,
    })
}

fn clone_status(is_cloned: bool) -> String {
    if is_cloned {
        "present".to_string()
    } else {
        "missing".to_string()
    }
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

fn repo_store_error_response(error: RepoStoreError) -> axum::response::Response {
    let status = match &error {
        RepoStoreError::InvalidRepoId(_) | RepoStoreError::InvalidPathSegment { .. } => {
            StatusCode::BAD_REQUEST
        }
        RepoStoreError::NotAllowedForProject { .. } => StatusCode::FORBIDDEN,
        RepoStoreError::CloneMissing { .. } => StatusCode::NOT_FOUND,
        RepoStoreError::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        RepoStoreError::Unimplemented(_) => StatusCode::NOT_IMPLEMENTED,
    };
    let error = error.client_message();

    (status, Json(ErrorResponse { error })).into_response()
}

async fn repo_entry_response(
    state: &AppState,
    ctx: &RepoAccessContext,
    repo_id: &RepoId,
) -> RepoAllowlistEntryResponse {
    let is_cloned = state
        .repo_clone_cache
        .is_cloned(&ctx.project.tenant_id, repo_id)
        .await;

    RepoAllowlistEntryResponse {
        repo_id: repo_id.as_str().to_string(),
        clone_status: clone_status(is_cloned),
        added_by: None,
        added_at: None,
        last_used_at: None,
    }
}

pub async fn list_project_repos_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(query): Query<ListQuery>,
    Path(project): Path<String>,
) -> impl IntoResponse {
    let ctx = match repo_access_context(&project) {
        Ok(ctx) => ctx,
        Err(message) => return bad_request_response(message),
    };
    // T6b-C5: refuse cross-tenant enumeration.
    if !enforce_project_tenant(&principal, &ctx.project) {
        return crate::errors::tenant_scope_mismatch_error().into_response();
    }
    let repo_ids = state.project_repo_access.list_for_project(&ctx).await;
    let paged_repo_ids: Vec<_> = repo_ids
        .into_iter()
        .skip(query.offset())
        .take(query.limit())
        .collect();
    let mut repos = Vec::with_capacity(paged_repo_ids.len());

    for repo_id in paged_repo_ids {
        repos.push(repo_entry_response(state.as_ref(), &ctx, &repo_id).await);
    }

    (
        StatusCode::OK,
        Json(RepoAllowlistListResponse { project, repos }),
    )
        .into_response()
}

pub async fn add_project_repo_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(project): Path<String>,
    Json(body): Json<AddRepoRequest>,
) -> impl IntoResponse {
    let ctx = match repo_access_context(&project) {
        Ok(ctx) => ctx,
        Err(message) => return bad_request_response(message),
    };
    // T6b-C5: refuse cross-tenant repo registration.
    if !enforce_project_tenant(&principal, &ctx.project) {
        return crate::errors::tenant_scope_mismatch_error().into_response();
    }
    let repo_id = match RepoId::parse(body.repo_id) {
        Ok(repo_id) => repo_id,
        Err(error) => return bad_request_response(error.to_string()),
    };
    let was_cloned = state
        .repo_clone_cache
        .is_cloned(&ctx.project.tenant_id, &repo_id)
        .await;

    let actor = ActorRef::Operator {
        operator_id: operator_id_from_principal(&principal),
    };

    if let Err(error) = state.project_repo_access.allow(&ctx, &repo_id, actor).await {
        return repo_store_error_response(error);
    }

    if let Err(error) = state
        .repo_clone_cache
        .ensure_cloned(&ctx.project.tenant_id, &repo_id)
        .await
    {
        return repo_store_error_response(error);
    }

    let is_cloned = state
        .repo_clone_cache
        .is_cloned(&ctx.project.tenant_id, &repo_id)
        .await;
    let response = RepoMutationResponse {
        project,
        repo_id: repo_id.as_str().to_string(),
        allowlisted: true,
        clone_status: clone_status(is_cloned),
        clone_created: !was_cloned && is_cloned,
    };

    (StatusCode::OK, Json(response)).into_response()
}

pub async fn get_project_repo_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path((project, owner, repo)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let ctx = match repo_access_context(&project) {
        Ok(ctx) => ctx,
        Err(message) => return bad_request_response(message),
    };
    // T6b-C5: refuse cross-tenant read.
    if !enforce_project_tenant(&principal, &ctx.project) {
        return crate::errors::tenant_scope_mismatch_error().into_response();
    }
    let repo_id = match RepoId::parse(format!("{owner}/{repo}")) {
        Ok(repo_id) => repo_id,
        Err(error) => return bad_request_response(error.to_string()),
    };
    let allowlisted = state.project_repo_access.is_allowed(&ctx, &repo_id).await;
    let is_cloned = state
        .repo_clone_cache
        .is_cloned(&ctx.project.tenant_id, &repo_id)
        .await;

    Json(RepoDetailResponse {
        project,
        repo_id: repo_id.as_str().to_string(),
        allowlisted,
        clone_status: clone_status(is_cloned),
        added_by: None,
        added_at: None,
        last_used_at: None,
        recent_sandbox_usage: Vec::new(),
        recent_register_repo_decisions: Vec::new(),
    })
    .into_response()
}

pub async fn delete_project_repo_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path((project, owner, repo)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let ctx = match repo_access_context(&project) {
        Ok(ctx) => ctx,
        Err(message) => return bad_request_response(message),
    };
    // T6b-C5: refuse cross-tenant repo revocation.
    if !enforce_project_tenant(&principal, &ctx.project) {
        return crate::errors::tenant_scope_mismatch_error().into_response();
    }
    let repo_id = match RepoId::parse(format!("{owner}/{repo}")) {
        Ok(repo_id) => repo_id,
        Err(error) => return bad_request_response(error.to_string()),
    };
    let actor = ActorRef::Operator {
        operator_id: operator_id_from_principal(&principal),
    };

    if let Err(error) = state
        .project_repo_access
        .revoke(&ctx, &repo_id, actor)
        .await
    {
        return repo_store_error_response(error);
    }

    StatusCode::NO_CONTENT.into_response()
}
