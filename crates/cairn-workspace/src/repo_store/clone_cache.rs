use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::TenantId;

use crate::sandbox::{RepoId, SandboxId};

type CloneKey = (TenantId, RepoId);
type CloneGuard = Arc<Mutex<()>>;

#[derive(Debug)]
pub struct RepoCloneCache {
    base_dir: PathBuf,
    clone_locks: RwLock<HashMap<CloneKey, CloneGuard>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefreshOutcome {
    pub old_head: String,
    pub new_head: String,
    pub drifted_sandboxes: Vec<SandboxId>,
}

impl RepoCloneCache {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            clone_locks: RwLock::new(HashMap::new()),
        }
    }

    pub(crate) fn path(&self, tenant: &TenantId, repo_id: &RepoId) -> PathBuf {
        let (owner, repo) = repo_id.owner_and_repo();
        self.base_dir.join(tenant.as_str()).join(owner).join(repo)
    }

    pub async fn ensure_cloned(
        &self,
        tenant: &TenantId,
        repo_id: &RepoId,
    ) -> Result<(), crate::error::RepoStoreError> {
        let clone_lock = self.clone_lock(tenant, repo_id);
        let _guard = clone_lock.lock().expect("clone lock poisoned");
        let path = self.path(tenant, repo_id);

        if Self::is_clone_layout_present(&path) {
            return Ok(());
        }

        let git_dir = path.join(".git");
        fs::create_dir_all(&git_dir).map_err(|error| {
            crate::error::RepoStoreError::io("create clone directories", git_dir.clone(), error)
        })?;

        let head = format!(
            "init-{}-{}",
            repo_id.as_str().replace('/', "-"),
            Self::now_millis()
        );
        let head_path = Self::head_path(&path);
        fs::write(&head_path, format!("{head}\n")).map_err(|error| {
            crate::error::RepoStoreError::io("write clone head", head_path.clone(), error)
        })?;

        let origin_path = path.join("origin.txt");
        fs::write(&origin_path, repo_id.as_str()).map_err(|error| {
            crate::error::RepoStoreError::io("write clone origin", origin_path.clone(), error)
        })?;

        let marker_path = Self::marker_path(&path);
        fs::write(&marker_path, b"locked\n").map_err(|error| {
            crate::error::RepoStoreError::io("write clone marker", marker_path.clone(), error)
        })?;

        Self::set_readonly_recursive(&path, true)?;
        Ok(())
    }

    pub async fn is_cloned(&self, tenant: &TenantId, repo_id: &RepoId) -> bool {
        Self::is_clone_layout_present(&self.path(tenant, repo_id))
    }

    pub async fn refresh(
        &self,
        tenant: &TenantId,
        repo_id: &RepoId,
    ) -> Result<RefreshOutcome, crate::error::RepoStoreError> {
        let clone_lock = self.clone_lock(tenant, repo_id);
        let _guard = clone_lock.lock().expect("clone lock poisoned");
        let path = self.path(tenant, repo_id);

        if !Self::is_clone_layout_present(&path) {
            return Err(crate::error::RepoStoreError::CloneMissing {
                tenant: tenant.clone(),
                repo_id: repo_id.clone(),
            });
        }

        Self::set_readonly_recursive(&path, false)?;

        let head_path = Self::head_path(&path);
        let old_head = fs::read_to_string(&head_path)
            .map_err(|error| {
                crate::error::RepoStoreError::io("read clone head", head_path.clone(), error)
            })?
            .trim()
            .to_string();
        let new_head = format!(
            "refresh-{}-{}",
            repo_id.as_str().replace('/', "-"),
            Self::now_millis()
        );
        fs::write(&head_path, format!("{new_head}\n")).map_err(|error| {
            crate::error::RepoStoreError::io("write refreshed head", head_path.clone(), error)
        })?;
        Self::set_readonly_recursive(&path, true)?;

        Ok(RefreshOutcome {
            old_head,
            new_head,
            drifted_sandboxes: Vec::new(),
        })
    }

    pub async fn cloned_set(&self) -> HashSet<(TenantId, RepoId)> {
        let mut clones = HashSet::new();
        let tenant_dirs = match fs::read_dir(&self.base_dir) {
            Ok(entries) => entries,
            Err(_) => return clones,
        };

        for tenant_entry in tenant_dirs.flatten() {
            let Ok(file_type) = tenant_entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }

            let tenant = TenantId::new(tenant_entry.file_name().to_string_lossy().into_owned());
            let owner_dirs = match fs::read_dir(tenant_entry.path()) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for owner_entry in owner_dirs.flatten() {
                let Ok(owner_type) = owner_entry.file_type() else {
                    continue;
                };
                if !owner_type.is_dir() {
                    continue;
                }

                let owner = owner_entry.file_name().to_string_lossy().into_owned();
                let repo_dirs = match fs::read_dir(owner_entry.path()) {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };

                for repo_entry in repo_dirs.flatten() {
                    let Ok(repo_type) = repo_entry.file_type() else {
                        continue;
                    };
                    if !repo_type.is_dir() {
                        continue;
                    }

                    if Self::is_clone_layout_present(&repo_entry.path()) {
                        let repo_name = repo_entry.file_name().to_string_lossy().into_owned();
                        clones
                            .insert((tenant.clone(), RepoId::new(format!("{owner}/{repo_name}"))));
                    }
                }
            }
        }

        clones
    }

    pub async fn delete(
        &self,
        tenant: &TenantId,
        repo_id: &RepoId,
    ) -> Result<(), crate::error::RepoStoreError> {
        let clone_lock = self.clone_lock(tenant, repo_id);
        let _guard = clone_lock.lock().expect("clone lock poisoned");
        let path = self.path(tenant, repo_id);

        if !path.exists() {
            return Ok(());
        }

        Self::set_readonly_recursive(&path, false)?;
        fs::remove_dir_all(&path).map_err(|error| {
            crate::error::RepoStoreError::io("delete clone", path.clone(), error)
        })?;
        Self::prune_empty_parents(&self.base_dir, &path)?;
        Ok(())
    }

    fn clone_lock(&self, tenant: &TenantId, repo_id: &RepoId) -> CloneGuard {
        let key = (tenant.clone(), repo_id.clone());
        if let Some(existing) = self
            .clone_locks
            .read()
            .expect("clone lock map poisoned")
            .get(&key)
            .cloned()
        {
            return existing;
        }

        self.clone_locks
            .write()
            .expect("clone lock map poisoned")
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    fn head_path(path: &Path) -> PathBuf {
        path.join(".git").join("HEAD")
    }

    fn marker_path(path: &Path) -> PathBuf {
        path.join(".cairn-clone.locked")
    }

    fn is_clone_layout_present(path: &Path) -> bool {
        Self::head_path(path).is_file() && Self::marker_path(path).is_file()
    }

    fn set_readonly_recursive(
        path: &Path,
        readonly: bool,
    ) -> Result<(), crate::error::RepoStoreError> {
        if !path.exists() {
            return Ok(());
        }

        if !readonly {
            Self::set_readonly(path, false)?;
        }

        if path.is_dir() {
            let entries = fs::read_dir(path).map_err(|error| {
                crate::error::RepoStoreError::io("read clone tree", path.to_path_buf(), error)
            })?;
            for entry in entries {
                let entry = entry.map_err(|error| {
                    crate::error::RepoStoreError::io("read clone entry", path.to_path_buf(), error)
                })?;
                Self::set_readonly_recursive(&entry.path(), readonly)?;
            }
        }

        if readonly {
            Self::set_readonly(path, true)?;
        }

        Ok(())
    }

    fn set_readonly(path: &Path, readonly: bool) -> Result<(), crate::error::RepoStoreError> {
        let mut permissions = fs::metadata(path)
            .map_err(|error| {
                crate::error::RepoStoreError::io("read permissions", path.to_path_buf(), error)
            })?
            .permissions();
        permissions.set_readonly(readonly);
        fs::set_permissions(path, permissions).map_err(|error| {
            crate::error::RepoStoreError::io("set permissions", path.to_path_buf(), error)
        })
    }

    fn prune_empty_parents(
        root: &Path,
        starting_path: &Path,
    ) -> Result<(), crate::error::RepoStoreError> {
        let mut current = starting_path.parent();
        while let Some(path) = current {
            if path == root {
                break;
            }

            let is_empty = fs::read_dir(path)
                .map_err(|error| {
                    crate::error::RepoStoreError::io(
                        "scan parent for prune",
                        path.to_path_buf(),
                        error,
                    )
                })?
                .next()
                .is_none();
            if !is_empty {
                break;
            }

            fs::remove_dir(path).map_err(|error| {
                crate::error::RepoStoreError::io("prune empty parent", path.to_path_buf(), error)
            })?;
            current = path.parent();
        }

        Ok(())
    }

    fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_millis() as u64
    }
}

impl Default for RepoCloneCache {
    fn default() -> Self {
        Self::new(std::env::temp_dir().join("cairn-workspace-repos"))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    use cairn_domain::TenantId;

    use super::RepoCloneCache;
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

    #[tokio::test]
    async fn ensure_cloned_uses_tenant_scoped_paths_and_tracks_set() {
        let temp_dir = TestDir::new("clone-cache");
        let cache = RepoCloneCache::new(&temp_dir.path);
        let tenant = TenantId::new("tenant-a");
        let repo = RepoId::new("octocat/hello-world");

        cache.ensure_cloned(&tenant, &repo).await.unwrap();
        let clone_path = cache.path(&tenant, &repo);

        assert!(clone_path.ends_with("tenant-a/octocat/hello-world"));
        assert!(cache.is_cloned(&tenant, &repo).await);
        assert!(fs::metadata(clone_path.join(".git").join("HEAD"))
            .unwrap()
            .permissions()
            .readonly());

        let clones = cache.cloned_set().await;
        assert!(clones.contains(&(tenant, repo)));
    }

    #[tokio::test]
    async fn ensure_cloned_is_idempotent_and_refresh_relocks_head() {
        let temp_dir = TestDir::new("clone-refresh");
        let cache = RepoCloneCache::new(&temp_dir.path);
        let tenant = TenantId::new("tenant-a");
        let repo = RepoId::new("octocat/hello-world");

        cache.ensure_cloned(&tenant, &repo).await.unwrap();
        let clone_path = cache.path(&tenant, &repo);
        let head_path = clone_path.join(".git").join("HEAD");
        let original_head = fs::read_to_string(&head_path).unwrap();

        cache.ensure_cloned(&tenant, &repo).await.unwrap();
        assert_eq!(fs::read_to_string(&head_path).unwrap(), original_head);

        let refresh = cache.refresh(&tenant, &repo).await.unwrap();

        assert_eq!(refresh.old_head, original_head.trim());
        assert_ne!(refresh.old_head, refresh.new_head);
        assert!(fs::metadata(&head_path).unwrap().permissions().readonly());
        assert_eq!(
            fs::read_to_string(&head_path).unwrap().trim(),
            refresh.new_head
        );
    }

    #[tokio::test]
    async fn delete_removes_clone_and_prunes_empty_owner_dirs() {
        let temp_dir = TestDir::new("clone-delete");
        let cache = RepoCloneCache::new(&temp_dir.path);
        let tenant = TenantId::new("tenant-a");
        let repo = RepoId::new("octocat/hello-world");

        cache.ensure_cloned(&tenant, &repo).await.unwrap();
        let clone_path = cache.path(&tenant, &repo);
        let owner_dir = clone_path.parent().unwrap().to_path_buf();

        cache.delete(&tenant, &repo).await.unwrap();

        assert!(!clone_path.exists());
        assert!(!owner_dir.exists());
        assert!(!cache.is_cloned(&tenant, &repo).await);
    }
}
