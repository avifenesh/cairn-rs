//! Sandbox configuration helpers.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::errors::AppApiError;

pub(crate) fn default_sandbox_strategy() -> cairn_workspace::SandboxStrategyRequest {
    #[cfg(target_os = "linux")]
    {
        cairn_workspace::SandboxStrategyRequest::Preferred(
            cairn_workspace::SandboxStrategy::Overlay,
        )
    }

    #[cfg(not(target_os = "linux"))]
    {
        cairn_workspace::SandboxStrategyRequest::Preferred(
            cairn_workspace::SandboxStrategy::Reflink,
        )
    }
}

pub(crate) fn default_repo_sandbox_policy(
    repo_id: cairn_workspace::RepoId,
) -> cairn_workspace::SandboxPolicy {
    cairn_workspace::SandboxPolicy {
        strategy: default_sandbox_strategy(),
        base: cairn_workspace::SandboxBase::Repo {
            repo_id,
            starting_ref: None,
        },
        credentials: Vec::new(),
        network_egress: None,
        memory_limit_bytes: None,
        cpu_weight: None,
        disk_quota_bytes: None,
        wall_clock_limit: None,
        on_resource_exhaustion: cairn_domain::OnExhaustion::Destroy,
        preserve_on_failure: true,
        required_host_caps: cairn_workspace::HostCapabilityRequirements::default(),
    }
}

pub(crate) fn workspace_error_response(err: cairn_workspace::WorkspaceError) -> Response {
    use cairn_workspace::{RepoStoreError, WorkspaceError};

    match err {
        WorkspaceError::RepoStore(error) => {
            let status = match &error {
                RepoStoreError::InvalidRepoId(_) | RepoStoreError::InvalidPathSegment { .. } => {
                    StatusCode::BAD_REQUEST
                }
                RepoStoreError::NotAllowedForProject { .. } => StatusCode::FORBIDDEN,
                RepoStoreError::CloneMissing { .. } => StatusCode::NOT_FOUND,
                RepoStoreError::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,
                RepoStoreError::Unimplemented(_) => StatusCode::NOT_IMPLEMENTED,
            };
            AppApiError::new(status, "repo_store_error", error.client_message()).into_response()
        }
        WorkspaceError::ProviderUnavailable { .. } => AppApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "sandbox_provider_unavailable",
            err.to_string(),
        )
        .into_response(),
        WorkspaceError::SandboxNotFound { .. } => {
            AppApiError::new(StatusCode::NOT_FOUND, "sandbox_not_found", err.to_string())
                .into_response()
        }
        WorkspaceError::InvalidSandboxStateTransition { .. }
        | WorkspaceError::ResourceLimitMissing { .. } => AppApiError::new(
            StatusCode::CONFLICT,
            "sandbox_state_conflict",
            err.to_string(),
        )
        .into_response(),
        WorkspaceError::BaseRevisionDrift { .. } => AppApiError::new(
            StatusCode::CONFLICT,
            "sandbox_base_revision_drift",
            err.to_string(),
        )
        .into_response(),
        WorkspaceError::SandboxOperation { operation, .. } => AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "sandbox_operation_failed",
            format!("sandbox {operation} failed"),
        )
        .into_response(),
        WorkspaceError::Unimplemented(message) => {
            AppApiError::new(StatusCode::NOT_IMPLEMENTED, "unimplemented", message).into_response()
        }
    }
}
