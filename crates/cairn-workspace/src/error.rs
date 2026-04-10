use std::fmt::{Display, Formatter};
use std::path::PathBuf;

use cairn_domain::{ProjectKey, ResourceDimension, RunId, TenantId};

use crate::sandbox::{RepoId, SandboxState, SandboxStrategy};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceError {
    Unimplemented(&'static str),
    RepoStore(RepoStoreError),
    ProviderUnavailable {
        strategy: SandboxStrategy,
    },
    SandboxNotFound {
        run_id: RunId,
    },
    InvalidSandboxStateTransition {
        run_id: RunId,
        from: SandboxState,
        to: SandboxState,
    },
    ResourceLimitMissing {
        run_id: RunId,
        dimension: ResourceDimension,
    },
    BaseRevisionDrift {
        run_id: RunId,
        expected: String,
        actual: String,
    },
    SandboxOperation {
        run_id: RunId,
        operation: &'static str,
        message: String,
    },
}

impl WorkspaceError {
    pub fn unimplemented(message: &'static str) -> Self {
        Self::Unimplemented(message)
    }

    pub fn sandbox_op(
        run_id: &RunId,
        operation: &'static str,
        error: impl std::fmt::Display,
    ) -> Self {
        Self::SandboxOperation {
            run_id: run_id.clone(),
            operation,
            message: error.to_string(),
        }
    }
}

impl Display for WorkspaceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceError::Unimplemented(message) => write!(f, "unimplemented: {message}"),
            WorkspaceError::RepoStore(error) => write!(f, "{error}"),
            WorkspaceError::ProviderUnavailable { strategy } => {
                write!(f, "sandbox provider {strategy:?} is unavailable")
            }
            WorkspaceError::SandboxNotFound { run_id } => {
                write!(f, "sandbox for run {run_id} was not found")
            }
            WorkspaceError::InvalidSandboxStateTransition { run_id, from, to } => write!(
                f,
                "invalid sandbox transition for run {run_id}: {from:?} -> {to:?}"
            ),
            WorkspaceError::ResourceLimitMissing { run_id, dimension } => write!(
                f,
                "sandbox for run {run_id} has no configured limit for {dimension:?}"
            ),
            WorkspaceError::BaseRevisionDrift {
                run_id,
                expected,
                actual,
            } => write!(
                f,
                "sandbox base revision drift detected for run {run_id}: expected {expected}, actual {actual}"
            ),
            WorkspaceError::SandboxOperation {
                run_id,
                operation,
                message,
            } => write!(
                f,
                "sandbox operation {operation} failed for run {run_id}: {message}"
            ),
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl From<RepoStoreError> for WorkspaceError {
    fn from(value: RepoStoreError) -> Self {
        Self::RepoStore(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoStoreError {
    NotAllowedForProject {
        project: ProjectKey,
        repo_id: RepoId,
    },
    CloneMissing {
        tenant: TenantId,
        repo_id: RepoId,
    },
    Io {
        action: &'static str,
        path: PathBuf,
        message: String,
    },
    Unimplemented(&'static str),
}

impl Display for RepoStoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoStoreError::NotAllowedForProject { project, repo_id } => {
                write!(
                    f,
                    "repo {repo_id} is not allowlisted for project {:?}",
                    project
                )
            }
            RepoStoreError::CloneMissing { tenant, repo_id } => {
                write!(f, "clone {repo_id} is missing for tenant {tenant}")
            }
            RepoStoreError::Io {
                action,
                path,
                message,
            } => {
                write!(f, "{action} failed at {}: {message}", path.display())
            }
            RepoStoreError::Unimplemented(message) => write!(f, "unimplemented: {message}"),
        }
    }
}

impl std::error::Error for RepoStoreError {}

impl RepoStoreError {
    pub fn io(action: &'static str, path: PathBuf, error: impl std::fmt::Display) -> Self {
        Self::Io {
            action,
            path,
            message: error.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SweepError {
    RepoStore(RepoStoreError),
    ActiveSandboxQuery(String),
    Unimplemented(&'static str),
}

impl Display for SweepError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SweepError::RepoStore(error) => write!(f, "{error}"),
            SweepError::ActiveSandboxQuery(message) => {
                write!(f, "active sandbox repo query failed: {message}")
            }
            SweepError::Unimplemented(message) => write!(f, "unimplemented: {message}"),
        }
    }
}

impl std::error::Error for SweepError {}

impl From<RepoStoreError> for SweepError {
    fn from(value: RepoStoreError) -> Self {
        Self::RepoStore(value)
    }
}
