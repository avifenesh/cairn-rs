use std::fmt::{Display, Formatter};
use std::path::PathBuf;

use cairn_domain::{ProjectKey, TenantId};

use crate::sandbox::RepoId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceError {
    Unimplemented(&'static str),
    RepoStore(RepoStoreError),
}

impl WorkspaceError {
    pub fn unimplemented(message: &'static str) -> Self {
        Self::Unimplemented(message)
    }
}

impl Display for WorkspaceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceError::Unimplemented(message) => write!(f, "unimplemented: {message}"),
            WorkspaceError::RepoStore(error) => write!(f, "{error}"),
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
    Unimplemented(&'static str),
}

impl Display for SweepError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SweepError::Unimplemented(message) => write!(f, "unimplemented: {message}"),
        }
    }
}

impl std::error::Error for SweepError {}
