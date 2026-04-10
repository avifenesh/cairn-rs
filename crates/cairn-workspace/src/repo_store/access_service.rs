use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use cairn_domain::RepoAccessContext;

use crate::sandbox::RepoId;

#[derive(Debug, Default)]
pub struct ProjectRepoAccessService {
    pub allowed: RwLock<HashMap<String, HashSet<RepoId>>>,
}

impl ProjectRepoAccessService {
    pub fn is_allowed(&self, ctx: &RepoAccessContext, repo_id: &RepoId) -> bool {
        let project_slot = format!(
            "{}/{}/{}",
            ctx.project.tenant_id.as_str(),
            ctx.project.workspace_id.as_str(),
            ctx.project.project_id.as_str()
        );

        self.allowed
            .read()
            .ok()
            .and_then(|map| map.get(&project_slot).cloned())
            .map(|repos| repos.contains(repo_id))
            .unwrap_or(false)
    }
}
