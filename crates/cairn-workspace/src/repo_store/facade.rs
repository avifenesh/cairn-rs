use std::path::PathBuf;
use std::sync::Arc;

use cairn_domain::RepoAccessContext;

use crate::error::RepoStoreError;
use crate::repo_store::{ProjectRepoAccessService, RepoCloneCache};
use crate::sandbox::RepoId;

#[derive(Debug, Clone)]
pub struct RepoStore {
    cache: Arc<RepoCloneCache>,
    access: Arc<ProjectRepoAccessService>,
}

impl RepoStore {
    pub fn new(cache: Arc<RepoCloneCache>, access: Arc<ProjectRepoAccessService>) -> Self {
        Self { cache, access }
    }

    pub async fn resolve(
        &self,
        ctx: &RepoAccessContext,
        repo_id: &RepoId,
    ) -> Result<PathBuf, RepoStoreError> {
        repo_id.validate()?;
        if !self.access.is_allowed(ctx, repo_id).await {
            return Err(RepoStoreError::NotAllowedForProject {
                project: ctx.project.clone(),
                repo_id: repo_id.clone(),
            });
        }

        let tenant = &ctx.project.tenant_id;
        self.cache.ensure_cloned(tenant, repo_id).await?;
        Ok(self.cache.path(tenant, repo_id))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use cairn_domain::{ActorRef, OperatorId, ProjectKey, RepoAccessContext};

    use super::RepoStore;
    use crate::error::RepoStoreError;
    use crate::repo_store::{ProjectRepoAccessService, RepoCloneCache};
    use crate::sandbox::RepoId;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "cairn-workspace-{label}-{}-{unique}",
                process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn ctx(project: &str) -> RepoAccessContext {
        RepoAccessContext {
            project: ProjectKey::new("tenant-a", "workspace-a", project),
        }
    }

    fn actor() -> ActorRef {
        ActorRef::Operator {
            operator_id: OperatorId::new("op"),
        }
    }

    #[tokio::test]
    async fn resolve_rejects_before_touching_disk() {
        let temp_dir = TestDir::new("repo-store-deny");
        let cache = Arc::new(RepoCloneCache::new(&temp_dir.path));
        let access = Arc::new(ProjectRepoAccessService::new());
        let store = RepoStore::new(cache.clone(), access);
        let project_ctx = ctx("project-a");
        let repo = RepoId::new("octocat/hello-world");

        let error = store.resolve(&project_ctx, &repo).await.unwrap_err();

        assert_eq!(
            error,
            RepoStoreError::NotAllowedForProject {
                project: project_ctx.project.clone(),
                repo_id: repo.clone(),
            }
        );
        assert!(!cache.is_cloned(&project_ctx.project.tenant_id, &repo).await);
    }

    #[tokio::test]
    async fn resolve_ensures_clone_after_allowlist_passes() {
        let temp_dir = TestDir::new("repo-store-allow");
        let cache = Arc::new(RepoCloneCache::new(&temp_dir.path));
        let access = Arc::new(ProjectRepoAccessService::new());
        let store = RepoStore::new(cache.clone(), access.clone());
        let project_ctx = ctx("project-a");
        let repo = RepoId::new("octocat/hello-world");

        access.allow(&project_ctx, &repo, actor()).await.unwrap();
        let resolved = store.resolve(&project_ctx, &repo).await.unwrap();

        assert_eq!(resolved, cache.path(&project_ctx.project.tenant_id, &repo));
        assert!(cache.is_cloned(&project_ctx.project.tenant_id, &repo).await);
    }
}
