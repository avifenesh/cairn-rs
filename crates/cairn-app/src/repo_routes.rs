//! HTTP route handlers for project-scoped repo access — RFC 016.
//!
//! Routes:
//!   GET    /v1/projects/:project/repos
//!   POST   /v1/projects/:project/repos
//!   GET    /v1/projects/:project/repos/:owner/:repo
//!   DELETE /v1/projects/:project/repos/:owner/:repo

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use cairn_domain::{ActorRef, OperatorId, ProjectKey, RepoAccessContext};
use cairn_workspace::RepoId;
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

fn operator_id_from_state(_state: &AppState) -> OperatorId {
    // TODO: extract from auth context once wired.
    OperatorId::new("operator")
}

fn project_key_from_path(project: &str) -> ProjectKey {
    if let Some((tenant_id, workspace_id, project_id)) = crate::parse_project_scope(project) {
        return ProjectKey::new(tenant_id, workspace_id, project_id);
    }

    ProjectKey::new(
        crate::DEFAULT_TENANT_ID,
        crate::DEFAULT_WORKSPACE_ID,
        project,
    )
}

fn repo_access_context(project: &str) -> RepoAccessContext {
    RepoAccessContext {
        project: project_key_from_path(project),
    }
}

fn clone_status(is_cloned: bool) -> String {
    if is_cloned {
        "present".to_string()
    } else {
        "missing".to_string()
    }
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
    Path(project): Path<String>,
) -> impl IntoResponse {
    let ctx = repo_access_context(&project);
    let repo_ids = state.project_repo_access.list_for_project(&ctx).await;
    let mut repos = Vec::with_capacity(repo_ids.len());

    for repo_id in repo_ids {
        repos.push(repo_entry_response(state.as_ref(), &ctx, &repo_id).await);
    }

    Json(RepoAllowlistListResponse { project, repos })
}

pub async fn add_project_repo_handler(
    State(state): State<Arc<AppState>>,
    Path(project): Path<String>,
    Json(body): Json<AddRepoRequest>,
) -> impl IntoResponse {
    let ctx = repo_access_context(&project);
    let repo_id = RepoId::new(body.repo_id);
    let was_cloned = state
        .repo_clone_cache
        .is_cloned(&ctx.project.tenant_id, &repo_id)
        .await;

    let actor = ActorRef::Operator {
        operator_id: operator_id_from_state(&state),
    };

    if let Err(error) = state.project_repo_access.allow(&ctx, &repo_id, actor).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response();
    }

    if let Err(error) = state
        .repo_clone_cache
        .ensure_cloned(&ctx.project.tenant_id, &repo_id)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response();
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
    let ctx = repo_access_context(&project);
    let repo_id = RepoId::new(format!("{owner}/{repo}"));
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
}

pub async fn delete_project_repo_handler(
    State(state): State<Arc<AppState>>,
    Path((project, owner, repo)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let ctx = repo_access_context(&project);
    let repo_id = RepoId::new(format!("{owner}/{repo}"));
    let actor = ActorRef::Operator {
        operator_id: operator_id_from_state(&state),
    };

    if let Err(error) = state
        .project_repo_access
        .revoke(&ctx, &repo_id, actor)
        .await
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use cairn_api::bootstrap::BootstrapConfig;
    use cairn_domain::RepoAccessContext;
    use cairn_workspace::RepoId;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::{
        add_project_repo_handler, delete_project_repo_handler, get_project_repo_handler,
        list_project_repos_handler, AddRepoRequest, RepoAllowlistListResponse, RepoDetailResponse,
        RepoMutationResponse,
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
            project: super::project_key_from_path(project_id),
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
        let scoped = super::project_key_from_path("tenant-a/workspace-a/project-a");
        assert_eq!(scoped.tenant_id.as_str(), "tenant-a");
        assert_eq!(scoped.workspace_id.as_str(), "workspace-a");
        assert_eq!(scoped.project_id.as_str(), "project-a");

        let fallback = super::project_key_from_path("project-only");
        assert_eq!(fallback.tenant_id.as_str(), crate::DEFAULT_TENANT_ID);
        assert_eq!(fallback.workspace_id.as_str(), crate::DEFAULT_WORKSPACE_ID);
        assert_eq!(fallback.project_id.as_str(), "project-only");
    }
}
