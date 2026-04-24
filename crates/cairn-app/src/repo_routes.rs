//! HTTP route handlers for project-scoped repo access — RFC 016.
//!
//! Routes:
//!   GET    /v1/projects/:project/repos
//!   POST   /v1/projects/:project/repos
//!   GET    /v1/projects/:project/repos/:owner/:repo
//!   DELETE /v1/projects/:project/repos/:owner/:repo

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use cairn_api::auth::AuthPrincipal;
use cairn_domain::{ActorRef, OperatorId, ProjectKey, RepoAccessContext};
use cairn_workspace::{RepoId, RepoStoreError};
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Host identifier for the GitHub canonical `owner/repo` shape.
pub const HOST_GITHUB: &str = "github";
/// Host identifier for arbitrary local-filesystem paths.
pub const HOST_LOCAL_FS: &str = "local_fs";

/// In-memory per-project set of local-filesystem paths.
///
/// Parallel to `ProjectRepoAccessService` (which enforces the strict
/// `owner/repo` shape for GitHub repos). Local paths are opaque
/// directory strings — no `/` splitting, no clone cache, no RFC 016
/// allowlist semantics. Used purely so the multi-host UI can attach a
/// local directory as a pseudo-repo for dogfood.
#[derive(Debug, Default)]
pub struct ProjectLocalPaths {
    paths: RwLock<HashMap<ProjectKey, HashSet<String>>>,
}

impl ProjectLocalPaths {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow(&self, project: &ProjectKey, path: &str) {
        let mut guard = self.paths.write().expect("local_paths lock poisoned");
        guard
            .entry(project.clone())
            .or_default()
            .insert(path.to_owned());
    }

    pub fn revoke(&self, project: &ProjectKey, path: &str) -> bool {
        let mut guard = self.paths.write().expect("local_paths lock poisoned");
        if let Some(set) = guard.get_mut(project) {
            let removed = set.remove(path);
            if set.is_empty() {
                guard.remove(project);
            }
            return removed;
        }
        false
    }

    pub fn list(&self, project: &ProjectKey) -> Vec<String> {
        let mut paths = self
            .paths
            .read()
            .ok()
            .and_then(|m| m.get(project).cloned())
            .map(|s| s.into_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        paths.sort();
        paths
    }

    pub fn contains(&self, project: &ProjectKey, path: &str) -> bool {
        self.paths
            .read()
            .ok()
            .and_then(|m| m.get(project).map(|s| s.contains(path)))
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepoAllowlistEntryResponse {
    pub repo_id: String,
    pub clone_status: String,
    pub added_by: Option<String>,
    pub added_at: Option<u64>,
    pub last_used_at: Option<u64>,
    /// Git host for this entry. Defaults to `"github"` for backward
    /// compatibility with pre-PR persisted events.
    #[serde(default = "default_host")]
    pub host: String,
}

fn default_host() -> String {
    HOST_GITHUB.into()
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
    #[serde(default = "default_host")]
    pub host: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepoMutationResponse {
    pub project: String,
    pub repo_id: String,
    pub allowlisted: bool,
    pub clone_status: String,
    pub clone_created: bool,
    #[serde(default = "default_host")]
    pub host: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ErrorResponse {
    error: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AddRepoRequest {
    pub repo_id: String,
    /// Git host — defaults to `"github"`. Accepted: `"github"`, `"local_fs"`.
    /// `"gitlab"`, `"gitea"`, `"confluence"` are recognised but return 501.
    #[serde(default = "default_host")]
    pub host: String,
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
        host: HOST_GITHUB.into(),
    }
}

fn local_path_entry(path: &str) -> RepoAllowlistEntryResponse {
    RepoAllowlistEntryResponse {
        repo_id: path.to_owned(),
        // Local directories are always "present" if they remain on
        // disk; revalidating here would require I/O per list row, so
        // we surface a dedicated status label and let the UI treat it
        // as non-cloneable.
        clone_status: "local".to_owned(),
        added_by: None,
        added_at: None,
        last_used_at: None,
        host: HOST_LOCAL_FS.into(),
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
    let local_paths = state.project_local_paths.list(&ctx.project);
    // Paginate the concatenated list so the UI sees a single stable
    // ordering: GitHub repos first (sorted by access_service), then
    // local paths (sorted by ProjectLocalPaths::list).
    let total = repo_ids.len() + local_paths.len();
    let mut repos = Vec::with_capacity(total.min(query.limit()));
    let mut skipped = 0usize;
    let mut taken = 0usize;
    let limit = query.limit();
    let offset = query.offset();

    for repo_id in repo_ids {
        if skipped < offset {
            skipped += 1;
            continue;
        }
        if taken >= limit {
            break;
        }
        repos.push(repo_entry_response(state.as_ref(), &ctx, &repo_id).await);
        taken += 1;
    }

    for path in local_paths {
        if skipped < offset {
            skipped += 1;
            continue;
        }
        if taken >= limit {
            break;
        }
        repos.push(local_path_entry(&path));
        taken += 1;
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

    match body.host.as_str() {
        HOST_GITHUB => add_github_repo(state, &ctx, project, &body.repo_id, &principal).await,
        HOST_LOCAL_FS => add_local_fs_path(state, &ctx, project, &body.repo_id),
        "gitlab" | "gitea" | "confluence" => (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: format!(
                    "host \"{}\" is recognised but not yet implemented",
                    body.host
                ),
            }),
        )
            .into_response(),
        other => bad_request_response(format!(
            "unknown host \"{other}\". Valid: github, local_fs, gitlab, gitea, confluence"
        )),
    }
}

async fn add_github_repo(
    state: Arc<AppState>,
    ctx: &RepoAccessContext,
    project: String,
    raw_repo_id: &str,
    principal: &AuthPrincipal,
) -> axum::response::Response {
    let repo_id = match RepoId::parse(raw_repo_id.to_owned()) {
        Ok(repo_id) => repo_id,
        Err(error) => return bad_request_response(error.to_string()),
    };
    let was_cloned = state
        .repo_clone_cache
        .is_cloned(&ctx.project.tenant_id, &repo_id)
        .await;

    let actor = ActorRef::Operator {
        operator_id: operator_id_from_principal(principal),
    };

    if let Err(error) = state.project_repo_access.allow(ctx, &repo_id, actor).await {
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
        host: HOST_GITHUB.into(),
    };

    (StatusCode::OK, Json(response)).into_response()
}

fn add_local_fs_path(
    state: Arc<AppState>,
    ctx: &RepoAccessContext,
    project: String,
    path: &str,
) -> axum::response::Response {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return bad_request_response("local_fs repo_id must be a non-empty path");
    }
    // Reject traversal and relative paths up-front; a workspace operator
    // should only ever point cairn at an absolute directory they own.
    let p = std::path::Path::new(trimmed);
    if !p.is_absolute() {
        return bad_request_response("local_fs repo_id must be an absolute path");
    }
    if trimmed.contains("..") {
        return bad_request_response("local_fs repo_id must not contain `..` segments");
    }
    if !p.exists() {
        return bad_request_response(format!("local_fs path does not exist: {trimmed}"));
    }
    if !p.is_dir() {
        return bad_request_response(format!("local_fs path is not a directory: {trimmed}"));
    }

    let already = state.project_local_paths.contains(&ctx.project, trimmed);
    state.project_local_paths.allow(&ctx.project, trimmed);

    let response = RepoMutationResponse {
        project,
        repo_id: trimmed.to_owned(),
        allowlisted: true,
        clone_status: "local".to_owned(),
        clone_created: !already,
        host: HOST_LOCAL_FS.into(),
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
        host: HOST_GITHUB.into(),
    })
    .into_response()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeleteLocalPathRequest {
    pub path: String,
}

/// DELETE /v1/projects/:project/local-paths — detach a local_fs path.
///
/// `DELETE /repos/:owner/:repo` can't carry arbitrary filesystem paths
/// (slashes break the 2-segment capture), so local_fs detach needs a
/// dedicated endpoint with the path in the body.
pub async fn delete_project_local_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(project): Path<String>,
    Json(body): Json<DeleteLocalPathRequest>,
) -> impl IntoResponse {
    let ctx = match repo_access_context(&project) {
        Ok(ctx) => ctx,
        Err(message) => return bad_request_response(message),
    };
    if !enforce_project_tenant(&principal, &ctx.project) {
        return crate::errors::tenant_scope_mismatch_error().into_response();
    }
    let trimmed = body.path.trim();
    if trimmed.is_empty() {
        return bad_request_response("path must be non-empty");
    }
    let removed = state.project_local_paths.revoke(&ctx.project, trimmed);
    if removed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("no local_fs path '{trimmed}' attached to this project"),
            }),
        )
            .into_response()
    }
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
