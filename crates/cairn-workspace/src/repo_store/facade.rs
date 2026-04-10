use std::sync::Arc;

use crate::repo_store::{ProjectRepoAccessService, RepoCloneCache};

#[derive(Debug, Clone)]
pub struct RepoStore {
    pub cache: Arc<RepoCloneCache>,
    pub access: Arc<ProjectRepoAccessService>,
}
