use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::RwLock;

use cairn_domain::TenantId;

use crate::sandbox::{RepoId, SandboxId};

#[derive(Debug)]
pub struct RepoCloneCache {
    pub base_dir: PathBuf,
    pub clone_locks: RwLock<HashMap<(TenantId, RepoId), ()>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefreshOutcome {
    pub old_head: String,
    pub new_head: String,
    pub drifted_sandboxes: Vec<SandboxId>,
}

impl RepoCloneCache {
    pub fn path(&self, tenant: &TenantId, repo_id: &RepoId) -> PathBuf {
        let (owner, repo) = repo_id.owner_and_repo();
        self.base_dir.join(tenant.as_str()).join(owner).join(repo)
    }

    pub fn cloned_set(&self) -> HashSet<(TenantId, RepoId)> {
        HashSet::new()
    }
}
