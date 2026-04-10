use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{CheckpointKind, RunId};
use serde::{Deserialize, Serialize};

use crate::error::WorkspaceError;
use crate::providers::SandboxProvider;
use crate::sandbox::{
    DestroyResult, ProvisionedSandbox, RepoId, SandboxBase, SandboxCheckpoint, SandboxHandle,
    SandboxId, SandboxMetadata, SandboxPolicy, SandboxStrategy,
};

const REFLINK_STRATEGY_NAME: &str = "reflink";
const REFLINK_STATE_FILE: &str = "reflink-state.json";
const STRATEGY_FILE: &str = "strategy";
const META_FILE: &str = "meta.json";
const ROOT_DIR: &str = "root";
const EMPTY_BASE_DIR: &str = "base.empty";

pub trait ReflinkCloneDriver: Send + Sync + 'static {
    fn clone_tree(&self, src: &Path, dst: &Path) -> Result<(), String>;
}

pub trait ReflinkRepoSource: Send + Sync + 'static {
    fn resolve_repo(
        &self,
        repo_id: &RepoId,
        starting_ref: Option<&str>,
    ) -> Result<ResolvedReflinkBase, WorkspaceError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedReflinkBase {
    pub source_dir: PathBuf,
    pub base_revision: Option<String>,
    pub branch: Option<String>,
}

#[derive(Default)]
struct UnsupportedRepoSource;

impl ReflinkRepoSource for UnsupportedRepoSource {
    fn resolve_repo(
        &self,
        _repo_id: &RepoId,
        _starting_ref: Option<&str>,
    ) -> Result<ResolvedReflinkBase, WorkspaceError> {
        Err(WorkspaceError::unimplemented(
            "reflink repo source integration is not wired yet",
        ))
    }
}

#[derive(Debug, Default)]
struct SystemReflinkCloneDriver;

impl SystemReflinkCloneDriver {
    fn clone_tree_recursive(src: &Path, dst: &Path) -> Result<(), String> {
        let metadata = fs::symlink_metadata(src).map_err(|error| error.to_string())?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            let target = fs::read_link(src).map_err(|error| error.to_string())?;
            Self::clone_symlink(&target, src, dst).map_err(|error| error.to_string())?;
            return Ok(());
        }

        if metadata.is_dir() {
            fs::create_dir_all(dst).map_err(|error| error.to_string())?;
            let entries = fs::read_dir(src).map_err(|error| error.to_string())?;
            for entry in entries {
                let entry = entry.map_err(|error| error.to_string())?;
                Self::clone_tree_recursive(&entry.path(), &dst.join(entry.file_name()))?;
            }
            return Ok(());
        }

        reflink_copy::reflink_or_copy(src, dst)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    #[cfg(unix)]
    fn clone_symlink(target: &Path, _src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, dst)
    }

    #[cfg(windows)]
    fn clone_symlink(target: &Path, src: &Path, dst: &Path) -> std::io::Result<()> {
        let is_dir = fs::metadata(src)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        if is_dir {
            std::os::windows::fs::symlink_dir(target, dst)
        } else {
            std::os::windows::fs::symlink_file(target, dst)
        }
    }
}

impl ReflinkCloneDriver for SystemReflinkCloneDriver {
    fn clone_tree(&self, src: &Path, dst: &Path) -> Result<(), String> {
        if dst.exists() {
            return Err(format!("destination already exists: {}", dst.display()));
        }
        Self::clone_tree_recursive(src, dst)
    }
}

#[derive(Clone)]
pub struct ReflinkProvider {
    base_dir: PathBuf,
    clone_driver: Arc<dyn ReflinkCloneDriver>,
    repo_source: Arc<dyn ReflinkRepoSource>,
}

impl Debug for ReflinkProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReflinkProvider")
            .field("base_dir", &self.base_dir)
            .finish()
    }
}

impl ReflinkProvider {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self::with_dependencies(
            base_dir,
            Arc::new(SystemReflinkCloneDriver),
            Arc::new(UnsupportedRepoSource),
        )
    }

    pub fn with_dependencies(
        base_dir: impl Into<PathBuf>,
        clone_driver: Arc<dyn ReflinkCloneDriver>,
        repo_source: Arc<dyn ReflinkRepoSource>,
    ) -> Self {
        Self {
            base_dir: base_dir.into(),
            clone_driver,
            repo_source,
        }
    }

    fn sandbox_id_for(run_id: &RunId) -> SandboxId {
        SandboxId::new(format!("sbx-{}", run_id.as_str()))
    }

    fn sandbox_root_for(&self, run_id: &RunId) -> PathBuf {
        self.base_dir.join(Self::sandbox_id_for(run_id).as_str())
    }

    fn root_dir(root: &Path) -> PathBuf {
        root.join(ROOT_DIR)
    }

    fn state_path(root: &Path) -> PathBuf {
        root.join(REFLINK_STATE_FILE)
    }

    fn strategy_path(root: &Path) -> PathBuf {
        root.join(STRATEGY_FILE)
    }

    fn metadata_path(root: &Path) -> PathBuf {
        root.join(META_FILE)
    }

    fn empty_base_dir(root: &Path) -> PathBuf {
        root.join(EMPTY_BASE_DIR)
    }

    fn prepare_sandbox_root(&self, run_id: &RunId, root: &Path) -> Result<(), WorkspaceError> {
        fs::create_dir_all(root)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "create_reflink_root", error))
    }

    fn resolve_base(
        &self,
        run_id: &RunId,
        root: &Path,
        base: &SandboxBase,
    ) -> Result<ResolvedReflinkBase, WorkspaceError> {
        match base {
            SandboxBase::Repo {
                repo_id,
                starting_ref,
            } => self
                .repo_source
                .resolve_repo(repo_id, starting_ref.as_deref()),
            SandboxBase::Directory { path } => Ok(ResolvedReflinkBase {
                source_dir: path.clone(),
                base_revision: None,
                branch: None,
            }),
            SandboxBase::Empty => {
                let source_dir = Self::empty_base_dir(root);
                fs::create_dir_all(&source_dir).map_err(|error| {
                    WorkspaceError::sandbox_op(run_id, "create_reflink_empty_base", error)
                })?;
                Ok(ResolvedReflinkBase {
                    source_dir,
                    base_revision: None,
                    branch: None,
                })
            }
        }
    }

    fn write_strategy_file(&self, run_id: &RunId, root: &Path) -> Result<(), WorkspaceError> {
        fs::write(Self::strategy_path(root), REFLINK_STRATEGY_NAME)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "write_reflink_strategy", error))
    }

    fn write_state(
        &self,
        run_id: &RunId,
        root: &Path,
        state: &ReflinkState,
    ) -> Result<(), WorkspaceError> {
        let encoded = serde_json::to_vec_pretty(state).map_err(|error| {
            WorkspaceError::sandbox_op(run_id, "serialize_reflink_state", error)
        })?;
        fs::write(Self::state_path(root), encoded)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "write_reflink_state", error))
    }

    fn load_state(&self, run_id: &RunId) -> Result<Option<ReflinkState>, WorkspaceError> {
        let path = Self::state_path(&self.sandbox_root_for(run_id));
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = fs::read(&path)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "read_reflink_state", error))?;
        let state = serde_json::from_slice(&bytes)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "decode_reflink_state", error))?;
        Ok(Some(state))
    }

    fn previous_snapshot_dirs(
        &self,
        run_id: &RunId,
        root: &Path,
    ) -> Result<Vec<PathBuf>, WorkspaceError> {
        let mut indexed = Vec::new();
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(WorkspaceError::sandbox_op(
                    run_id,
                    "read_reflink_root",
                    error,
                ))
            }
        };

        for entry in entries {
            let entry = entry
                .map_err(|error| WorkspaceError::sandbox_op(run_id, "read_reflink_entry", error))?;
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }

            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(index) = name
                .strip_prefix("root.prev.")
                .and_then(|value| value.parse::<usize>().ok())
            else {
                continue;
            };
            indexed.push((index, entry.path()));
        }

        indexed.sort_by_key(|(index, _)| *index);
        Ok(indexed.into_iter().map(|(_, path)| path).collect())
    }

    fn rotate_previous_snapshots(
        &self,
        run_id: &RunId,
        root: &Path,
    ) -> Result<PathBuf, WorkspaceError> {
        let mut previous = self.previous_snapshot_dirs(run_id, root)?;
        previous.reverse();
        for path in previous {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    WorkspaceError::sandbox_op(
                        run_id,
                        "parse_reflink_snapshot_name",
                        "invalid utf-8",
                    )
                })?;
            let index = name
                .strip_prefix("root.prev.")
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| {
                    WorkspaceError::sandbox_op(run_id, "parse_reflink_snapshot_index", name)
                })?;
            let target = root.join(format!("root.prev.{}", index + 1));
            fs::rename(&path, &target).map_err(|error| {
                WorkspaceError::sandbox_op(run_id, "rotate_reflink_snapshot", error)
            })?;
        }

        let snapshot = root.join("root.prev.0");
        if snapshot.exists() {
            fs::remove_dir_all(&snapshot).map_err(|error| {
                WorkspaceError::sandbox_op(run_id, "clear_reflink_snapshot", error)
            })?;
        }
        Ok(snapshot)
    }

    fn count_tree(&self, run_id: &RunId, root: &Path) -> Result<(u32, u64), WorkspaceError> {
        if !root.exists() {
            return Ok((0, 0));
        }

        if root.is_file() {
            let metadata = fs::metadata(root)
                .map_err(|error| WorkspaceError::sandbox_op(run_id, "stat_reflink_file", error))?;
            return Ok((1, metadata.len()));
        }

        let mut files_changed = 0;
        let mut bytes_written = 0;
        let entries = fs::read_dir(root)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "read_reflink_tree", error))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                WorkspaceError::sandbox_op(run_id, "read_reflink_tree_entry", error)
            })?;
            let (entry_files, entry_bytes) = self.count_tree(run_id, &entry.path())?;
            files_changed += entry_files;
            bytes_written += entry_bytes;
        }
        Ok((files_changed, bytes_written))
    }

    fn count_mutations(
        &self,
        run_id: &RunId,
        root: &Path,
        base: &SandboxBase,
    ) -> Result<(u32, u64), WorkspaceError> {
        if matches!(base, SandboxBase::Empty) {
            return self.count_tree(run_id, &Self::root_dir(root));
        }

        let mut files_changed = 0;
        let mut bytes_written = 0;
        for snapshot in self.previous_snapshot_dirs(run_id, root)? {
            let (snapshot_files, snapshot_bytes) = self.count_tree(run_id, &snapshot)?;
            files_changed += snapshot_files;
            bytes_written += snapshot_bytes;
        }
        Ok((files_changed, bytes_written))
    }

    fn sandbox_from_state(
        &self,
        root: &Path,
        state: &ReflinkState,
        is_resumed: bool,
    ) -> ProvisionedSandbox {
        ProvisionedSandbox {
            sandbox_id: state.sandbox_id.clone(),
            run_id: state.run_id.clone(),
            path: Self::root_dir(root),
            base: state.base.clone(),
            strategy: SandboxStrategy::Reflink,
            base_revision: state.base_revision.clone(),
            branch: state.branch.clone(),
            is_resumed,
            env: HashMap::from([("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())]),
        }
    }
}

#[async_trait]
impl SandboxProvider for ReflinkProvider {
    fn strategy(&self) -> SandboxStrategy {
        SandboxStrategy::Reflink
    }

    async fn provision(
        &self,
        run_id: &RunId,
        policy: &SandboxPolicy,
    ) -> Result<ProvisionedSandbox, WorkspaceError> {
        if let Some(existing) = self.reconnect(run_id).await? {
            return Ok(existing);
        }

        let root = self.sandbox_root_for(run_id);
        self.prepare_sandbox_root(run_id, &root)?;
        let resolved = self.resolve_base(run_id, &root, &policy.base)?;
        self.clone_driver
            .clone_tree(&resolved.source_dir, &Self::root_dir(&root))
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "clone_reflink_root", error))?;

        let state = ReflinkState {
            sandbox_id: Self::sandbox_id_for(run_id),
            run_id: run_id.clone(),
            base: policy.base.clone(),
            base_revision: resolved.base_revision.clone(),
            branch: resolved.branch.clone(),
        };
        self.write_strategy_file(run_id, &root)?;
        self.write_state(run_id, &root, &state)?;

        Ok(self.sandbox_from_state(&root, &state, false))
    }

    async fn reconnect(
        &self,
        run_id: &RunId,
    ) -> Result<Option<ProvisionedSandbox>, WorkspaceError> {
        let Some(state) = self.load_state(run_id)? else {
            return Ok(None);
        };

        let root = self.sandbox_root_for(run_id);
        let sandbox_root = Self::root_dir(&root);
        if !sandbox_root.is_dir() {
            return Err(WorkspaceError::sandbox_op(
                run_id,
                "reconnect_reflink_root",
                "reflink root directory missing",
            ));
        }

        Ok(Some(self.sandbox_from_state(&root, &state, true)))
    }

    async fn checkpoint(
        &self,
        run_id: &RunId,
        kind: CheckpointKind,
    ) -> Result<SandboxCheckpoint, WorkspaceError> {
        let state = self
            .load_state(run_id)?
            .ok_or_else(|| WorkspaceError::SandboxNotFound {
                run_id: run_id.clone(),
            })?;
        let root = self.sandbox_root_for(run_id);
        let snapshot = self.rotate_previous_snapshots(run_id, &root)?;
        self.clone_driver
            .clone_tree(&Self::root_dir(&root), &snapshot)
            .map_err(|error| {
                WorkspaceError::sandbox_op(run_id, "checkpoint_reflink_root", error)
            })?;

        Ok(SandboxCheckpoint {
            sandbox_id: state.sandbox_id,
            run_id: run_id.clone(),
            kind,
            rescue_ref: None,
            upper_snapshot: Some(snapshot),
        })
    }

    async fn restore(
        &self,
        from_checkpoint: &SandboxCheckpoint,
        new_run_id: &RunId,
        policy: &SandboxPolicy,
    ) -> Result<ProvisionedSandbox, WorkspaceError> {
        if let Some(existing) = self.reconnect(new_run_id).await? {
            return Ok(existing);
        }

        let snapshot = from_checkpoint.upper_snapshot.as_ref().ok_or_else(|| {
            WorkspaceError::sandbox_op(
                new_run_id,
                "restore_reflink_checkpoint",
                "missing checkpoint snapshot",
            )
        })?;
        if !snapshot.is_dir() {
            return Err(WorkspaceError::sandbox_op(
                new_run_id,
                "restore_reflink_checkpoint",
                "checkpoint snapshot directory missing",
            ));
        }

        let root = self.sandbox_root_for(new_run_id);
        self.prepare_sandbox_root(new_run_id, &root)?;
        self.clone_driver
            .clone_tree(snapshot, &Self::root_dir(&root))
            .map_err(|error| {
                WorkspaceError::sandbox_op(new_run_id, "restore_reflink_root", error)
            })?;

        let resolved = self.resolve_base(new_run_id, &root, &policy.base)?;
        let state = ReflinkState {
            sandbox_id: Self::sandbox_id_for(new_run_id),
            run_id: new_run_id.clone(),
            base: policy.base.clone(),
            base_revision: resolved.base_revision.clone(),
            branch: resolved.branch.clone(),
        };
        self.write_strategy_file(new_run_id, &root)?;
        self.write_state(new_run_id, &root, &state)?;

        Ok(self.sandbox_from_state(&root, &state, false))
    }

    async fn destroy(
        &self,
        run_id: &RunId,
        preserve: bool,
    ) -> Result<DestroyResult, WorkspaceError> {
        let root = self.sandbox_root_for(run_id);
        let state = self
            .load_state(run_id)?
            .ok_or_else(|| WorkspaceError::SandboxNotFound {
                run_id: run_id.clone(),
            })?;
        let (files_changed, bytes_written) = self.count_mutations(run_id, &root, &state.base)?;

        if !preserve && root.exists() {
            fs::remove_dir_all(&root).map_err(|error| {
                WorkspaceError::sandbox_op(run_id, "remove_reflink_sandbox", error)
            })?;
        }

        Ok(DestroyResult {
            sandbox_id: state.sandbox_id,
            files_changed,
            bytes_written,
        })
    }

    async fn list(&self) -> Result<Vec<SandboxHandle>, WorkspaceError> {
        let entries = match fs::read_dir(&self.base_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(WorkspaceError::sandbox_op(
                    &RunId::new("_"),
                    "list_reflink_sandboxes",
                    error,
                ))
            }
        };

        let mut handles = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                WorkspaceError::sandbox_op(&RunId::new("_"), "read_reflink_listing", error)
            })?;
            let metadata_path = Self::metadata_path(&entry.path());
            if !metadata_path.is_file() {
                continue;
            }
            let bytes = fs::read(&metadata_path).map_err(|error| {
                WorkspaceError::sandbox_op(&RunId::new("_"), "read_reflink_metadata", error)
            })?;
            let metadata: SandboxMetadata = serde_json::from_slice(&bytes).map_err(|error| {
                WorkspaceError::sandbox_op(&RunId::new("_"), "decode_reflink_metadata", error)
            })?;
            if metadata.strategy == SandboxStrategy::Reflink {
                handles.push(SandboxHandle { metadata });
            }
        }
        Ok(handles)
    }

    async fn heartbeat(&self, run_id: &RunId) -> Result<(), WorkspaceError> {
        if self.load_state(run_id)?.is_none() {
            return Err(WorkspaceError::SandboxNotFound {
                run_id: run_id.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ReflinkState {
    sandbox_id: SandboxId,
    run_id: RunId,
    base: SandboxBase,
    base_revision: Option<String>,
    branch: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use cairn_domain::{CheckpointKind, RunId};

    use super::{ReflinkCloneDriver, ReflinkProvider, ReflinkRepoSource, ResolvedReflinkBase};
    use crate::error::WorkspaceError;
    use crate::providers::SandboxProvider;
    use crate::sandbox::{
        HostCapabilityRequirements, RepoId, SandboxBase, SandboxMetadata, SandboxPolicy,
        SandboxState, SandboxStrategy, SandboxStrategyRequest,
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CloneCall {
        src: PathBuf,
        dst: PathBuf,
    }

    #[derive(Debug, Default)]
    struct TestCloneDriver {
        calls: Mutex<Vec<CloneCall>>,
    }

    impl TestCloneDriver {
        fn calls(&self) -> Vec<CloneCall> {
            self.calls.lock().expect("clone call log poisoned").clone()
        }

        fn copy_recursive(src: &Path, dst: &Path) -> Result<(), String> {
            let metadata = fs::symlink_metadata(src).map_err(|error| error.to_string())?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                let target = fs::read_link(src).map_err(|error| error.to_string())?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, dst).map_err(|error| error.to_string())?;
                #[cfg(windows)]
                {
                    let is_dir = fs::metadata(src)
                        .map(|metadata| metadata.is_dir())
                        .unwrap_or(false);
                    if is_dir {
                        std::os::windows::fs::symlink_dir(&target, dst)
                            .map_err(|error| error.to_string())?;
                    } else {
                        std::os::windows::fs::symlink_file(&target, dst)
                            .map_err(|error| error.to_string())?;
                    }
                }
                return Ok(());
            }

            if metadata.is_dir() {
                fs::create_dir_all(dst).map_err(|error| error.to_string())?;
                let entries = fs::read_dir(src).map_err(|error| error.to_string())?;
                for entry in entries {
                    let entry = entry.map_err(|error| error.to_string())?;
                    Self::copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
                }
                return Ok(());
            }

            fs::copy(src, dst).map_err(|error| error.to_string())?;
            Ok(())
        }
    }

    impl ReflinkCloneDriver for TestCloneDriver {
        fn clone_tree(&self, src: &Path, dst: &Path) -> Result<(), String> {
            self.calls
                .lock()
                .expect("clone call log poisoned")
                .push(CloneCall {
                    src: src.to_path_buf(),
                    dst: dst.to_path_buf(),
                });
            if dst.exists() {
                return Err(format!("destination already exists: {}", dst.display()));
            }
            Self::copy_recursive(src, dst)
        }
    }

    #[derive(Debug, Default)]
    struct TestRepoSource {
        repos: Mutex<HashMap<RepoId, PathBuf>>,
    }

    impl TestRepoSource {
        fn insert_repo(&self, repo_id: RepoId, path: PathBuf) {
            self.repos
                .lock()
                .expect("repo source poisoned")
                .insert(repo_id, path);
        }
    }

    impl ReflinkRepoSource for TestRepoSource {
        fn resolve_repo(
            &self,
            repo_id: &RepoId,
            starting_ref: Option<&str>,
        ) -> Result<ResolvedReflinkBase, WorkspaceError> {
            let path = self
                .repos
                .lock()
                .expect("repo source poisoned")
                .get(repo_id)
                .cloned()
                .ok_or_else(|| WorkspaceError::unimplemented("test repo source missing repo"))?;
            let head_path = path.join(".git").join("HEAD");
            let base_revision = match fs::read_to_string(&head_path) {
                Ok(head) => Some(head.trim().to_string()),
                Err(_) => starting_ref.map(|value| value.to_string()),
            };
            Ok(ResolvedReflinkBase {
                source_dir: path,
                base_revision,
                branch: starting_ref.map(|value| value.to_string()),
            })
        }
    }

    fn unique_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cairn-reflink-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("test temp dir should be creatable");
        dir
    }

    fn run_id(value: &str) -> RunId {
        RunId::new(value)
    }

    fn directory_policy(path: PathBuf) -> SandboxPolicy {
        SandboxPolicy {
            strategy: SandboxStrategyRequest::Force(SandboxStrategy::Reflink),
            base: SandboxBase::Directory { path },
            credentials: Vec::new(),
            network_egress: None,
            memory_limit_bytes: None,
            cpu_weight: None,
            disk_quota_bytes: None,
            wall_clock_limit: None,
            on_resource_exhaustion: cairn_domain::OnExhaustion::Destroy,
            preserve_on_failure: true,
            required_host_caps: HostCapabilityRequirements::default(),
        }
    }

    fn repo_policy(repo_id: RepoId, starting_ref: Option<&str>) -> SandboxPolicy {
        SandboxPolicy {
            strategy: SandboxStrategyRequest::Force(SandboxStrategy::Reflink),
            base: SandboxBase::Repo {
                repo_id,
                starting_ref: starting_ref.map(|value| value.to_string()),
            },
            credentials: Vec::new(),
            network_egress: None,
            memory_limit_bytes: None,
            cpu_weight: None,
            disk_quota_bytes: None,
            wall_clock_limit: None,
            on_resource_exhaustion: cairn_domain::OnExhaustion::Destroy,
            preserve_on_failure: true,
            required_host_caps: HostCapabilityRequirements::default(),
        }
    }

    fn empty_policy() -> SandboxPolicy {
        SandboxPolicy {
            strategy: SandboxStrategyRequest::Force(SandboxStrategy::Reflink),
            base: SandboxBase::Empty,
            credentials: Vec::new(),
            network_egress: None,
            memory_limit_bytes: None,
            cpu_weight: None,
            disk_quota_bytes: None,
            wall_clock_limit: None,
            on_resource_exhaustion: cairn_domain::OnExhaustion::Destroy,
            preserve_on_failure: true,
            required_host_caps: HostCapabilityRequirements::default(),
        }
    }

    fn provider_with(
        base_dir: PathBuf,
        clone_driver: Arc<TestCloneDriver>,
        repo_source: Arc<TestRepoSource>,
    ) -> ReflinkProvider {
        ReflinkProvider::with_dependencies(base_dir, clone_driver, repo_source)
    }

    fn create_repo_dir(root: &Path, head: &str) -> PathBuf {
        let repo = root.join("repo");
        fs::create_dir_all(repo.join(".git")).expect("repo git dir should be creatable");
        fs::write(repo.join(".git").join("HEAD"), format!("{head}\n"))
            .expect("repo head should be writable");
        fs::write(repo.join("README.md"), "hello from repo").expect("repo file should be writable");
        repo
    }

    fn write_reflink_metadata(root: &Path, run_id: &RunId, strategy: SandboxStrategy) {
        let metadata = SandboxMetadata {
            sandbox_id: crate::sandbox::SandboxId::new(format!("sbx-{}", run_id.as_str())),
            run_id: run_id.clone(),
            task_id: None,
            project: cairn_domain::ProjectKey::new("tenant-a", "workspace-a", "project-a"),
            strategy,
            state: SandboxState::Ready,
            base_rev: None,
            repo_id: None,
            path: root.join("root"),
            pid: None,
            created_at: 1,
            heartbeat_at: 1,
            policy_hash: "policy:test".to_string(),
        };
        fs::write(
            root.join("meta.json"),
            serde_json::to_vec_pretty(&metadata).expect("metadata should serialize"),
        )
        .expect("metadata should be writable");
    }

    #[tokio::test]
    async fn provision_creates_reflink_root_for_directory_base() {
        let base_dir = unique_test_dir("provision");
        let source_dir = base_dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("notes.txt"), "hello").unwrap();

        let clone_driver = Arc::new(TestCloneDriver::default());
        let provider = provider_with(
            base_dir.clone(),
            clone_driver.clone(),
            Arc::new(TestRepoSource::default()),
        );
        let run = run_id("run-reflink-provision");

        let sandbox = provider
            .provision(&run, &directory_policy(source_dir.clone()))
            .await
            .unwrap();

        let root = base_dir.join("sbx-run-reflink-provision");
        assert_eq!(sandbox.path, root.join("root"));
        assert_eq!(
            fs::read_to_string(root.join("root").join("notes.txt")).unwrap(),
            "hello"
        );
        assert!(root.join("reflink-state.json").is_file());
        assert_eq!(
            fs::read_to_string(root.join("strategy")).unwrap(),
            "reflink"
        );

        let calls = clone_driver.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].src, source_dir);
        assert_eq!(calls[0].dst, root.join("root"));
    }

    #[tokio::test]
    async fn reconnect_uses_existing_root_without_drift_detection() {
        let base_dir = unique_test_dir("reconnect");
        let repo_dir = create_repo_dir(&base_dir, "abc123");
        let repo_id = RepoId::new("octocat/hello");

        let clone_driver = Arc::new(TestCloneDriver::default());
        let repo_source = Arc::new(TestRepoSource::default());
        repo_source.insert_repo(repo_id.clone(), repo_dir.clone());
        let provider = provider_with(base_dir.clone(), clone_driver, repo_source);
        let run = run_id("run-reflink-reconnect");

        provider
            .provision(&run, &repo_policy(repo_id, Some("main")))
            .await
            .unwrap();

        let sandbox_root = base_dir.join("sbx-run-reflink-reconnect").join("root");
        assert_eq!(
            fs::read_to_string(sandbox_root.join("README.md")).unwrap(),
            "hello from repo"
        );

        fs::write(repo_dir.join(".git").join("HEAD"), "def456\n").unwrap();
        fs::write(repo_dir.join("README.md"), "mutated source").unwrap();

        let sandbox = provider.reconnect(&run).await.unwrap().unwrap();
        assert!(sandbox.is_resumed);
        assert_eq!(
            fs::read_to_string(sandbox_root.join("README.md")).unwrap(),
            "hello from repo"
        );
    }

    #[tokio::test]
    async fn checkpoint_and_restore_clone_prior_mutations() {
        let base_dir = unique_test_dir("restore");
        let source_dir = base_dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("notes.txt"), "seed").unwrap();

        let clone_driver = Arc::new(TestCloneDriver::default());
        let provider = provider_with(
            base_dir.clone(),
            clone_driver.clone(),
            Arc::new(TestRepoSource::default()),
        );
        let run = run_id("run-reflink-checkpoint");

        provider
            .provision(&run, &directory_policy(source_dir))
            .await
            .unwrap();
        let root = base_dir.join("sbx-run-reflink-checkpoint");
        fs::write(root.join("root").join("draft.txt"), "v1").unwrap();

        let checkpoint = provider
            .checkpoint(&run, CheckpointKind::Intent)
            .await
            .unwrap();
        assert_eq!(checkpoint.upper_snapshot, Some(root.join("root.prev.0")));

        let restored = provider
            .restore(&checkpoint, &run_id("run-reflink-restore"), &empty_policy())
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(restored.path.join("draft.txt")).unwrap(),
            "v1"
        );
    }

    #[tokio::test]
    async fn destroy_counts_empty_base_mutations_and_removes_root() {
        let base_dir = unique_test_dir("destroy");
        let clone_driver = Arc::new(TestCloneDriver::default());
        let provider = provider_with(
            base_dir.clone(),
            clone_driver,
            Arc::new(TestRepoSource::default()),
        );
        let run = run_id("run-reflink-destroy");

        provider.provision(&run, &empty_policy()).await.unwrap();
        let root = base_dir.join("sbx-run-reflink-destroy");
        fs::write(root.join("root").join("a.txt"), "one").unwrap();
        fs::write(root.join("root").join("b.txt"), "two").unwrap();

        let result = provider.destroy(&run, false).await.unwrap();
        assert_eq!(result.files_changed, 2);
        assert_eq!(result.bytes_written, 6);
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn list_uses_persisted_metadata_for_reflink_sandboxes() {
        let base_dir = unique_test_dir("list");
        let reflink_root = base_dir.join("sbx-run-list");
        fs::create_dir_all(&reflink_root).unwrap();
        write_reflink_metadata(&reflink_root, &run_id("run-list"), SandboxStrategy::Reflink);

        let overlay_root = base_dir.join("sbx-run-overlay");
        fs::create_dir_all(&overlay_root).unwrap();
        write_reflink_metadata(
            &overlay_root,
            &run_id("run-overlay"),
            SandboxStrategy::Overlay,
        );

        let provider = provider_with(
            base_dir,
            Arc::new(TestCloneDriver::default()),
            Arc::new(TestRepoSource::default()),
        );

        let handles = provider.list().await.unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].metadata.run_id, run_id("run-list"));
    }
}
