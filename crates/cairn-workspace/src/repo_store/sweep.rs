use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cairn_domain::TenantId;

use crate::error::SweepError;
use crate::repo_store::{ProjectRepoAccessService, RepoCloneCache};
use crate::sandbox::RepoId;

#[async_trait]
pub trait ActiveSandboxRepoSource: Send + Sync {
    async fn active_repo_references(&self) -> Result<HashSet<(TenantId, RepoId)>, SweepError>;
}

pub struct RepoCloneSweepTask {
    pub cache: Arc<RepoCloneCache>,
    pub access: Arc<ProjectRepoAccessService>,
    pub sandbox_source: Arc<dyn ActiveSandboxRepoSource>,
    pub interval: Duration,
}
