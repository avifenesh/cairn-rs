//! HTTP route handlers for project-scoped repo access — RFC 016.
//!
//! Routes:
//!   GET    /v1/projects/:project/repos
//!   POST   /v1/projects/:project/repos
//!   GET    /v1/projects/:project/repos/:owner/:repo
//!   DELETE /v1/projects/:project/repos/:owner/:repo

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
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

fn operator_id_from_state(_state: &AppState) -> OperatorId {
    // TODO: extract from auth context once wired.
    OperatorId::new("operator")
}

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
    Query(query): Query<ListQuery>,
    Path(project): Path<String>,
) -> impl IntoResponse {
    let ctx = match repo_access_context(&project) {
        Ok(ctx) => ctx,
        Err(message) => return bad_request_response(message),
    };
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
    Path(project): Path<String>,
    Json(body): Json<AddRepoRequest>,
) -> impl IntoResponse {
    let ctx = match repo_access_context(&project) {
        Ok(ctx) => ctx,
        Err(message) => return bad_request_response(message),
    };
    let repo_id = match RepoId::parse(body.repo_id) {
        Ok(repo_id) => repo_id,
        Err(error) => return bad_request_response(error.to_string()),
    };
    let was_cloned = state
        .repo_clone_cache
        .is_cloned(&ctx.project.tenant_id, &repo_id)
        .await;

    let actor = ActorRef::Operator {
        operator_id: operator_id_from_state(&state),
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
    Path((project, owner, repo)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let ctx = match repo_access_context(&project) {
        Ok(ctx) => ctx,
        Err(message) => return bad_request_response(message),
    };
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
    Path((project, owner, repo)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let ctx = match repo_access_context(&project) {
        Ok(ctx) => ctx,
        Err(message) => return bad_request_response(message),
    };
    let repo_id = match RepoId::parse(format!("{owner}/{repo}")) {
        Ok(repo_id) => repo_id,
        Err(error) => return bad_request_response(error.to_string()),
    };
    let actor = ActorRef::Operator {
        operator_id: operator_id_from_state(&state),
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

// Boots `AppBootstrap` via `BootstrapConfig`; the default Fabric build
// refuses to boot without HMAC env (fail-loud). Gated on
// `in-memory-runtime` so `cargo test -p cairn-app` stays green by default.
#[cfg(all(test, feature = "in-memory-runtime"))]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use cairn_api::bootstrap::BootstrapConfig;
    use cairn_domain::RepoAccessContext;
    use cairn_workspace::{RepoId, RepoStoreError};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::{
        add_project_repo_handler, delete_project_repo_handler, get_project_repo_handler,
        list_project_repos_handler, AddRepoRequest, ListQuery, RepoAllowlistListResponse,
        RepoDetailResponse, RepoMutationResponse,
    };
    use crate::AppState;

    fn unique_suffix() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    }

    fn project_ctx(project_id: &str) -> RepoAccessContext {
        RepoAccessContext {
            project: super::project_key_from_path(project_id).unwrap(),
        }
    }

    fn test_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route(
                "/v1/projects/:project/repos",
                get(list_project_repos_handler).post(add_project_repo_handler),
            )
            .route(
                "/v1/projects/:project/repos/:owner/:repo",
                get(get_project_repo_handler).delete(delete_project_repo_handler),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn repo_access_routes_preserve_clone_after_delete() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        let router = test_router(state.clone());
        let suffix = unique_suffix();
        let project = format!("repo-route-project-{suffix}");
        let repo_name = format!("hello-{suffix}");
        let repo_id = format!("octocat/{repo_name}");

        let add_request = Request::builder()
            .method("POST")
            .uri(format!("/v1/projects/{project}/repos"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&AddRepoRequest {
                    repo_id: repo_id.clone(),
                })
                .unwrap(),
            ))
            .unwrap();
        let add_response = router.clone().oneshot(add_request).await.unwrap();
        assert_eq!(add_response.status(), StatusCode::OK);
        let add_body = add_response.into_body().collect().await.unwrap().to_bytes();
        let add_result: RepoMutationResponse = serde_json::from_slice(&add_body).unwrap();
        assert_eq!(add_result.repo_id, repo_id);
        assert!(add_result.allowlisted);
        assert_eq!(add_result.clone_status, "present");

        let list_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/projects/{project}/repos"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list_response.status(), StatusCode::OK);
        let list_body = list_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let list_result: RepoAllowlistListResponse = serde_json::from_slice(&list_body).unwrap();
        assert_eq!(list_result.repos.len(), 1);
        assert_eq!(list_result.repos[0].repo_id, repo_id);

        let detail_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/projects/{project}/repos/octocat/{repo_name}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detail_response.status(), StatusCode::OK);
        let detail_body = detail_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let detail_result: RepoDetailResponse = serde_json::from_slice(&detail_body).unwrap();
        assert!(detail_result.allowlisted);
        assert_eq!(detail_result.clone_status, "present");

        let delete_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/projects/{project}/repos/octocat/{repo_name}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let ctx = project_ctx(&project);
        let repo = RepoId::new(repo_id.clone());
        assert!(!state.project_repo_access.is_allowed(&ctx, &repo).await);
        assert!(
            state
                .repo_clone_cache
                .is_cloned(&ctx.project.tenant_id, &repo)
                .await
        );
    }

    #[test]
    fn project_key_from_path_accepts_full_scope_or_project_id() {
        let scoped = super::project_key_from_path("tenant-a/workspace-a/project-a").unwrap();
        assert_eq!(scoped.tenant_id.as_str(), "tenant-a");
        assert_eq!(scoped.workspace_id.as_str(), "workspace-a");
        assert_eq!(scoped.project_id.as_str(), "project-a");

        let fallback = super::project_key_from_path("project-only").unwrap();
        assert_eq!(fallback.tenant_id.as_str(), crate::DEFAULT_TENANT_ID);
        assert_eq!(fallback.workspace_id.as_str(), crate::DEFAULT_WORKSPACE_ID);
        assert_eq!(fallback.project_id.as_str(), "project-only");
    }

    #[test]
    fn list_query_caps_page_size() {
        let query = ListQuery {
            limit: Some(500),
            offset: Some(7),
        };

        assert_eq!(query.limit(), 100);
        assert_eq!(query.offset(), 7);
    }

    #[tokio::test]
    async fn add_repo_rejects_invalid_repo_ids() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        let router = test_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/projects/project-a/repos")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&AddRepoRequest {
                            repo_id: "../escape".to_string(),
                        })
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn io_errors_are_sanitized_for_clients() {
        let response = super::repo_store_error_response(RepoStoreError::io(
            "write clone head",
            PathBuf::from("/tmp/secret/clone"),
            "permission denied",
        ));

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let error: super::ErrorResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(error.error, "write clone head failed");
        assert!(!error.error.contains("/tmp/secret/clone"));
    }
}
