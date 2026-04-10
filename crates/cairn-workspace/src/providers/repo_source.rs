use std::fs;
use std::sync::Arc;

use cairn_domain::TenantId;

use crate::error::{RepoStoreError, WorkspaceError};
use crate::providers::overlay::{OverlayRepoSource, ResolvedOverlayBase};
use crate::providers::reflink::{ReflinkRepoSource, ResolvedReflinkBase};
use crate::repo_store::RepoCloneCache;
use crate::sandbox::RepoId;

#[derive(Clone, Debug)]
pub struct RepoCloneCacheSource {
    cache: Arc<RepoCloneCache>,
}

impl RepoCloneCacheSource {
    pub fn new(cache: Arc<RepoCloneCache>) -> Self {
        Self { cache }
    }

    fn resolve_clone(
        &self,
        tenant: &TenantId,
        repo_id: &RepoId,
    ) -> Result<(std::path::PathBuf, Option<String>), WorkspaceError> {
        let path = self.cache.path(tenant, repo_id);
        let head_path = path.join(".git").join("HEAD");
        if !head_path.is_file() {
            return Err(WorkspaceError::RepoStore(RepoStoreError::CloneMissing {
                tenant: tenant.clone(),
                repo_id: repo_id.clone(),
            }));
        }

        let base_revision = Some(
            fs::read_to_string(&head_path)
                .map_err(|error| RepoStoreError::io("read clone head", head_path, error))?
                .trim()
                .to_string(),
        );

        Ok((path, base_revision))
    }
}

impl OverlayRepoSource for RepoCloneCacheSource {
    fn resolve_repo(
        &self,
        tenant: &TenantId,
        repo_id: &RepoId,
        _starting_ref: Option<&str>,
    ) -> Result<ResolvedOverlayBase, WorkspaceError> {
        let (lower_dir, base_revision) = self.resolve_clone(tenant, repo_id)?;
        Ok(ResolvedOverlayBase {
            lower_dir,
            base_revision,
            branch: None,
        })
    }
}

impl ReflinkRepoSource for RepoCloneCacheSource {
    fn resolve_repo(
        &self,
        tenant: &TenantId,
        repo_id: &RepoId,
        _starting_ref: Option<&str>,
    ) -> Result<ResolvedReflinkBase, WorkspaceError> {
        let (source_dir, base_revision) = self.resolve_clone(tenant, repo_id)?;
        Ok(ResolvedReflinkBase {
            source_dir,
            base_revision,
            branch: None,
        })
    }
}
