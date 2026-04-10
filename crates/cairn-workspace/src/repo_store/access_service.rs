use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use cairn_domain::{ActorRef, ProjectKey, RepoAccessContext};

use crate::error::RepoStoreError;
use crate::sandbox::RepoId;

#[derive(Debug, Default)]
pub struct ProjectRepoAccessService {
    allowed: RwLock<HashMap<ProjectKey, HashSet<RepoId>>>,
}

impl ProjectRepoAccessService {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn is_allowed(&self, ctx: &RepoAccessContext, repo_id: &RepoId) -> bool {
        if repo_id.validate().is_err() {
            return false;
        }
        self.allowed
            .read()
            .ok()
            .and_then(|map| map.get(&ctx.project).map(|repos| repos.contains(repo_id)))
            .unwrap_or(false)
    }

    pub async fn allow(
        &self,
        ctx: &RepoAccessContext,
        repo_id: &RepoId,
        _by: ActorRef,
    ) -> Result<(), RepoStoreError> {
        repo_id.validate()?;
        let mut guard = self.allowed.write().expect("allowlist lock poisoned");
        guard
            .entry(ctx.project.clone())
            .or_default()
            .insert(repo_id.clone());
        Ok(())
    }

    pub async fn revoke(
        &self,
        ctx: &RepoAccessContext,
        repo_id: &RepoId,
        _by: ActorRef,
    ) -> Result<(), RepoStoreError> {
        repo_id.validate()?;
        let mut guard = self.allowed.write().expect("allowlist lock poisoned");
        if let Some(repos) = guard.get_mut(&ctx.project) {
            repos.remove(repo_id);
            if repos.is_empty() {
                guard.remove(&ctx.project);
            }
        }
        Ok(())
    }

    pub async fn list_for_project(&self, ctx: &RepoAccessContext) -> Vec<RepoId> {
        let mut repos = self
            .allowed
            .read()
            .ok()
            .and_then(|map| map.get(&ctx.project).cloned())
            .map(|repos| repos.into_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        repos.sort();
        repos
    }

    pub async fn list_all(&self) -> HashMap<ProjectKey, Vec<RepoId>> {
        self.allowed
            .read()
            .expect("allowlist lock poisoned")
            .iter()
            .map(|(project, repos)| {
                let mut repos = repos.iter().cloned().collect::<Vec<_>>();
                repos.sort();
                (project.clone(), repos)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::ProjectRepoAccessService;
    use crate::sandbox::RepoId;
    use cairn_domain::{ActorRef, OperatorId, ProjectKey, RepoAccessContext};

    fn ctx(project: &str) -> RepoAccessContext {
        RepoAccessContext {
            project: ProjectKey::new("tenant", "workspace", project),
        }
    }

    fn actor() -> ActorRef {
        ActorRef::Operator {
            operator_id: OperatorId::new("op"),
        }
    }

    #[tokio::test]
    async fn allowlist_is_project_scoped() {
        let service = ProjectRepoAccessService::new();
        let repo = RepoId::new("org/repo");

        service
            .allow(&ctx("project-a"), &repo, actor())
            .await
            .unwrap();

        assert!(service.is_allowed(&ctx("project-a"), &repo).await);
        assert!(!service.is_allowed(&ctx("project-b"), &repo).await);
    }

    #[tokio::test]
    async fn revoke_removes_last_repo_and_cleans_slot() {
        let service = ProjectRepoAccessService::new();
        let repo = RepoId::new("org/repo");
        let project_ctx = ctx("project-a");

        service.allow(&project_ctx, &repo, actor()).await.unwrap();
        service.revoke(&project_ctx, &repo, actor()).await.unwrap();

        assert!(!service.is_allowed(&project_ctx, &repo).await);
        assert!(service.list_all().await.is_empty());
    }

    #[tokio::test]
    async fn list_all_returns_hashmap_keyed_by_project() {
        let service = ProjectRepoAccessService::new();
        let repo_a = RepoId::new("org/repo-a");
        let repo_b = RepoId::new("org/repo-b");
        let project_a = ctx("project-a");
        let project_b = ctx("project-b");

        service.allow(&project_a, &repo_b, actor()).await.unwrap();
        service.allow(&project_a, &repo_a, actor()).await.unwrap();
        service.allow(&project_b, &repo_b, actor()).await.unwrap();

        let all = service.list_all().await;

        assert_eq!(
            all.get(&project_a.project),
            Some(&vec![repo_a, repo_b.clone()])
        );
        assert_eq!(all.get(&project_b.project), Some(&vec![repo_b]));
    }
}
