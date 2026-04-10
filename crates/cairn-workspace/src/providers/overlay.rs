use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

const OVERLAY_STRATEGY_NAME: &str = "overlay_fs";
const OVERLAY_STATE_FILE: &str = "overlay-state.json";
const STRATEGY_FILE: &str = "strategy";
const META_FILE: &str = "meta.json";
const EMPTY_BASE_DIR: &str = "base.empty";

pub trait OverlayMountDriver: Send + Sync + 'static {
    fn mount(
        &self,
        lower_dirs: &[PathBuf],
        upper: &Path,
        work: &Path,
        merged: &Path,
    ) -> Result<(), String>;

    fn unmount(&self, merged: &Path) -> Result<(), String>;

    fn is_mounted(&self, merged: &Path) -> Result<bool, String>;
}

pub trait OverlayRepoSource: Send + Sync + 'static {
    fn resolve_repo(
        &self,
        repo_id: &RepoId,
        starting_ref: Option<&str>,
    ) -> Result<ResolvedOverlayBase, WorkspaceError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedOverlayBase {
    pub lower_dir: PathBuf,
    pub base_revision: Option<String>,
    pub branch: Option<String>,
}

#[derive(Default)]
struct UnsupportedRepoSource;

impl OverlayRepoSource for UnsupportedRepoSource {
    fn resolve_repo(
        &self,
        _repo_id: &RepoId,
        _starting_ref: Option<&str>,
    ) -> Result<ResolvedOverlayBase, WorkspaceError> {
        Err(WorkspaceError::unimplemented(
            "overlay repo source integration is not wired yet",
        ))
    }
}

#[derive(Debug, Default)]
struct SystemOverlayMountDriver;

impl OverlayMountDriver for SystemOverlayMountDriver {
    fn mount(
        &self,
        lower_dirs: &[PathBuf],
        upper: &Path,
        work: &Path,
        merged: &Path,
    ) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            let lowerdir = lower_dirs
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(":");
            let options = format!(
                "lowerdir={lowerdir},upperdir={},workdir={}",
                upper.display(),
                work.display()
            );
            let output = Command::new("mount")
                .arg("-t")
                .arg("overlay")
                .arg("overlay")
                .arg("-o")
                .arg(options)
                .arg(merged)
                .output()
                .map_err(|error| error.to_string())?;
            if output.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (lower_dirs, upper, work, merged);
            Err("overlay mounts are only supported on linux".to_string())
        }
    }

    fn unmount(&self, merged: &Path) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            let output = Command::new("umount")
                .arg(merged)
                .output()
                .map_err(|error| error.to_string())?;
            if output.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = merged;
            Err("overlay unmount is only supported on linux".to_string())
        }
    }

    fn is_mounted(&self, merged: &Path) -> Result<bool, String> {
        #[cfg(target_os = "linux")]
        {
            let needle = merged.display().to_string();
            let mountinfo =
                fs::read_to_string("/proc/self/mountinfo").map_err(|error| error.to_string())?;
            Ok(mountinfo.lines().any(|line| {
                line.split_whitespace()
                    .nth(4)
                    .map(|mount_point| mount_point == needle)
                    .unwrap_or(false)
            }))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = merged;
            Ok(false)
        }
    }
}

#[derive(Clone)]
pub struct OverlayProvider {
    base_dir: PathBuf,
    mount_driver: Arc<dyn OverlayMountDriver>,
    repo_source: Arc<dyn OverlayRepoSource>,
}

impl Debug for OverlayProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlayProvider")
            .field("base_dir", &self.base_dir)
            .finish()
    }
}

impl OverlayProvider {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self::with_dependencies(
            base_dir,
            Arc::new(SystemOverlayMountDriver),
            Arc::new(UnsupportedRepoSource),
        )
    }

    pub fn with_dependencies(
        base_dir: impl Into<PathBuf>,
        mount_driver: Arc<dyn OverlayMountDriver>,
        repo_source: Arc<dyn OverlayRepoSource>,
    ) -> Self {
        Self {
            base_dir: base_dir.into(),
            mount_driver,
            repo_source,
        }
    }

    fn sandbox_id_for(run_id: &RunId) -> SandboxId {
        SandboxId::new(format!("sbx-{}", run_id.as_str()))
    }

    fn sandbox_root_for(&self, run_id: &RunId) -> PathBuf {
        self.base_dir.join(Self::sandbox_id_for(run_id).as_str())
    }

    fn upper_dir(root: &Path) -> PathBuf {
        root.join("upper")
    }

    fn work_dir(root: &Path) -> PathBuf {
        root.join("work")
    }

    fn merged_dir(root: &Path) -> PathBuf {
        root.join("merged")
    }

    fn strategy_path(root: &Path) -> PathBuf {
        root.join(STRATEGY_FILE)
    }

    fn overlay_state_path(root: &Path) -> PathBuf {
        root.join(OVERLAY_STATE_FILE)
    }

    fn metadata_path(root: &Path) -> PathBuf {
        root.join(META_FILE)
    }

    fn empty_base_dir(root: &Path) -> PathBuf {
        root.join(EMPTY_BASE_DIR)
    }

    fn create_root_layout(&self, run_id: &RunId, root: &Path) -> Result<(), WorkspaceError> {
        fs::create_dir_all(root)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "create_root_layout", error))
    }

    fn prepare_mount_dirs(&self, run_id: &RunId, root: &Path) -> Result<(), WorkspaceError> {
        fs::create_dir_all(Self::upper_dir(root))
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "create_upper_dir", error))?;
        if Self::work_dir(root).exists() {
            fs::remove_dir_all(Self::work_dir(root))
                .map_err(|error| WorkspaceError::sandbox_op(run_id, "reset_work_dir", error))?;
        }
        fs::create_dir_all(Self::work_dir(root))
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "create_work_dir", error))?;
        fs::create_dir_all(Self::merged_dir(root))
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "create_merged_dir", error))
    }

    fn resolve_base(
        &self,
        run_id: &RunId,
        root: &Path,
        base: &SandboxBase,
    ) -> Result<ResolvedOverlayBase, WorkspaceError> {
        match base {
            SandboxBase::Repo {
                repo_id,
                starting_ref,
            } => self
                .repo_source
                .resolve_repo(repo_id, starting_ref.as_deref()),
            SandboxBase::Directory { path } => Ok(ResolvedOverlayBase {
                lower_dir: path.clone(),
                base_revision: None,
                branch: None,
            }),
            SandboxBase::Empty => {
                let empty = Self::empty_base_dir(root);
                fs::create_dir_all(&empty).map_err(|error| {
                    WorkspaceError::sandbox_op(run_id, "create_empty_lower_dir", error)
                })?;
                Ok(ResolvedOverlayBase {
                    lower_dir: empty,
                    base_revision: None,
                    branch: None,
                })
            }
        }
    }

    fn build_lower_dirs(
        &self,
        run_id: &RunId,
        root: &Path,
        base_lower: &Path,
    ) -> Result<Vec<PathBuf>, WorkspaceError> {
        let mut lower_dirs = self.previous_upper_dirs(run_id, root)?;
        lower_dirs.push(base_lower.to_path_buf());
        Ok(lower_dirs)
    }

    fn previous_upper_dirs(
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
                    "read_overlay_root",
                    error,
                ))
            }
        };

        for entry in entries {
            let entry = entry
                .map_err(|error| WorkspaceError::sandbox_op(run_id, "read_overlay_entry", error))?;
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }

            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(index) = name
                .strip_prefix("upper.prev.")
                .and_then(|value| value.parse::<usize>().ok())
            else {
                continue;
            };
            indexed.push((index, entry.path()));
        }

        indexed.sort_by_key(|(index, _)| *index);
        Ok(indexed.into_iter().map(|(_, path)| path).collect())
    }

    fn mount_overlay(
        &self,
        run_id: &RunId,
        root: &Path,
        base_lower: &Path,
    ) -> Result<(), WorkspaceError> {
        self.prepare_mount_dirs(run_id, root)?;
        let lower_dirs = self.build_lower_dirs(run_id, root, base_lower)?;
        self.mount_driver
            .mount(
                &lower_dirs,
                &Self::upper_dir(root),
                &Self::work_dir(root),
                &Self::merged_dir(root),
            )
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "mount_overlay", error))
    }

    fn maybe_unmount(&self, run_id: &RunId, root: &Path) -> Result<(), WorkspaceError> {
        let merged = Self::merged_dir(root);
        let mounted = self
            .mount_driver
            .is_mounted(&merged)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "check_mount", error))?;
        if mounted {
            self.mount_driver
                .unmount(&merged)
                .map_err(|error| WorkspaceError::sandbox_op(run_id, "unmount_overlay", error))?;
        }
        Ok(())
    }

    fn write_strategy_file(&self, run_id: &RunId, root: &Path) -> Result<(), WorkspaceError> {
        fs::write(Self::strategy_path(root), OVERLAY_STRATEGY_NAME)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "write_overlay_strategy", error))
    }

    fn write_state(
        &self,
        run_id: &RunId,
        root: &Path,
        state: &OverlayState,
    ) -> Result<(), WorkspaceError> {
        let encoded = serde_json::to_vec_pretty(state).map_err(|error| {
            WorkspaceError::sandbox_op(run_id, "serialize_overlay_state", error)
        })?;
        fs::write(Self::overlay_state_path(root), encoded)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "write_overlay_state", error))
    }

    fn load_state(&self, run_id: &RunId) -> Result<Option<OverlayState>, WorkspaceError> {
        let path = Self::overlay_state_path(&self.sandbox_root_for(run_id));
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = fs::read(&path)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "read_overlay_state", error))?;
        let state = serde_json::from_slice(&bytes)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "decode_overlay_state", error))?;
        Ok(Some(state))
    }

    fn rotate_previous_layers(
        &self,
        run_id: &RunId,
        root: &Path,
    ) -> Result<PathBuf, WorkspaceError> {
        let mut previous = self.previous_upper_dirs(run_id, root)?;
        previous.reverse();
        for path in previous {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    WorkspaceError::sandbox_op(run_id, "parse_previous_upper_name", "invalid utf-8")
                })?;
            let index = name
                .strip_prefix("upper.prev.")
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| {
                    WorkspaceError::sandbox_op(run_id, "parse_previous_upper_index", name)
                })?;
            let target = root.join(format!("upper.prev.{}", index + 1));
            fs::rename(&path, &target).map_err(|error| {
                WorkspaceError::sandbox_op(run_id, "rotate_previous_upper", error)
            })?;
        }

        let upper = Self::upper_dir(root);
        let snapshot = root.join("upper.prev.0");
        if snapshot.exists() {
            fs::remove_dir_all(&snapshot).map_err(|error| {
                WorkspaceError::sandbox_op(run_id, "clear_previous_snapshot", error)
            })?;
        }
        fs::rename(&upper, &snapshot)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "snapshot_current_upper", error))?;
        Ok(snapshot)
    }

    fn count_mutations(&self, run_id: &RunId, root: &Path) -> Result<(u32, u64), WorkspaceError> {
        let mut files_changed = 0;
        let mut bytes_written = 0;
        let mut roots = vec![Self::upper_dir(root)];
        roots.extend(self.previous_upper_dirs(run_id, root)?);

        for layer in roots {
            if !layer.exists() {
                continue;
            }
            let (layer_files, layer_bytes) = self.count_tree(run_id, &layer)?;
            files_changed += layer_files;
            bytes_written += layer_bytes;
        }

        Ok((files_changed, bytes_written))
    }

    fn count_tree(&self, run_id: &RunId, root: &Path) -> Result<(u32, u64), WorkspaceError> {
        if root.is_file() {
            let metadata = fs::metadata(root)
                .map_err(|error| WorkspaceError::sandbox_op(run_id, "stat_mutation_file", error))?;
            return Ok((1, metadata.len()));
        }

        let mut files_changed = 0;
        let mut bytes_written = 0;
        let entries = fs::read_dir(root)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "read_mutation_tree", error))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                WorkspaceError::sandbox_op(run_id, "read_mutation_entry", error)
            })?;
            let (entry_files, entry_bytes) = self.count_tree(run_id, &entry.path())?;
            files_changed += entry_files;
            bytes_written += entry_bytes;
        }
        Ok((files_changed, bytes_written))
    }

    fn sandbox_from_state(
        &self,
        root: &Path,
        state: &OverlayState,
        is_resumed: bool,
    ) -> ProvisionedSandbox {
        ProvisionedSandbox {
            sandbox_id: state.sandbox_id.clone(),
            run_id: state.run_id.clone(),
            path: Self::merged_dir(root),
            base: state.base.clone(),
            strategy: SandboxStrategy::Overlay,
            base_revision: state.base_revision.clone(),
            branch: state.branch.clone(),
            is_resumed,
            env: HashMap::from([("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())]),
        }
    }
}

#[async_trait]
impl SandboxProvider for OverlayProvider {
    fn strategy(&self) -> SandboxStrategy {
        SandboxStrategy::Overlay
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
        self.create_root_layout(run_id, &root)?;
        let resolved = self.resolve_base(run_id, &root, &policy.base)?;
        self.mount_overlay(run_id, &root, &resolved.lower_dir)?;

        let state = OverlayState {
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
        let resolved = self.resolve_base(run_id, &root, &state.base)?;
        if matches!(state.base, SandboxBase::Repo { .. }) {
            if let (Some(expected), Some(actual)) = (
                state.base_revision.as_deref(),
                resolved.base_revision.as_deref(),
            ) {
                if expected != actual {
                    return Err(WorkspaceError::BaseRevisionDrift {
                        run_id: run_id.clone(),
                        expected: expected.to_string(),
                        actual: actual.to_string(),
                    });
                }
            }
        }

        let merged = Self::merged_dir(&root);
        let mounted = self
            .mount_driver
            .is_mounted(&merged)
            .map_err(|error| WorkspaceError::sandbox_op(run_id, "check_mount", error))?;
        if !mounted {
            self.mount_overlay(run_id, &root, &resolved.lower_dir)?;
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
        let resolved = self.resolve_base(run_id, &root, &state.base)?;

        self.maybe_unmount(run_id, &root)?;
        let snapshot = self.rotate_previous_layers(run_id, &root)?;
        self.mount_overlay(run_id, &root, &resolved.lower_dir)?;

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
            WorkspaceError::sandbox_op(new_run_id, "restore_checkpoint", "missing upper snapshot")
        })?;
        if !snapshot.is_dir() {
            return Err(WorkspaceError::sandbox_op(
                new_run_id,
                "restore_checkpoint",
                "upper snapshot directory missing",
            ));
        }

        let root = self.sandbox_root_for(new_run_id);
        self.create_root_layout(new_run_id, &root)?;
        let resolved = self.resolve_base(new_run_id, &root, &policy.base)?;
        self.prepare_mount_dirs(new_run_id, &root)?;

        let lower_dirs = vec![snapshot.clone(), resolved.lower_dir.clone()];
        self.mount_driver
            .mount(
                &lower_dirs,
                &Self::upper_dir(&root),
                &Self::work_dir(&root),
                &Self::merged_dir(&root),
            )
            .map_err(|error| {
                WorkspaceError::sandbox_op(new_run_id, "mount_overlay_restore", error)
            })?;

        let state = OverlayState {
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

        let (files_changed, bytes_written) = self.count_mutations(run_id, &root)?;
        self.maybe_unmount(run_id, &root)?;

        if !preserve && root.exists() {
            fs::remove_dir_all(&root).map_err(|error| {
                WorkspaceError::sandbox_op(run_id, "remove_overlay_sandbox", error)
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
                    "list_overlay_sandboxes",
                    error,
                ))
            }
        };

        let mut handles = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                WorkspaceError::sandbox_op(&RunId::new("_"), "read_overlay_listing", error)
            })?;
            let metadata_path = Self::metadata_path(&entry.path());
            if !metadata_path.is_file() {
                continue;
            }
            let bytes = fs::read(&metadata_path).map_err(|error| {
                WorkspaceError::sandbox_op(&RunId::new("_"), "read_overlay_metadata", error)
            })?;
            let metadata: SandboxMetadata = serde_json::from_slice(&bytes).map_err(|error| {
                WorkspaceError::sandbox_op(&RunId::new("_"), "decode_overlay_metadata", error)
            })?;
            if metadata.strategy == SandboxStrategy::Overlay {
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
struct OverlayState {
    sandbox_id: SandboxId,
    run_id: RunId,
    base: SandboxBase,
    base_revision: Option<String>,
    branch: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use cairn_domain::{CheckpointKind, RunId};

    use super::{OverlayMountDriver, OverlayProvider, OverlayRepoSource, ResolvedOverlayBase};
    use crate::error::WorkspaceError;
    use crate::providers::SandboxProvider;
    use crate::sandbox::{
        HostCapabilityRequirements, RepoId, SandboxBase, SandboxMetadata, SandboxPolicy,
        SandboxState, SandboxStrategy, SandboxStrategyRequest,
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct MountCall {
        lower_dirs: Vec<PathBuf>,
        upper: PathBuf,
        work: PathBuf,
        merged: PathBuf,
    }

    #[derive(Debug, Default)]
    struct TestMountDriver {
        mounted: Mutex<HashSet<PathBuf>>,
        calls: Mutex<Vec<MountCall>>,
        unmounts: Mutex<Vec<PathBuf>>,
    }

    impl TestMountDriver {
        fn mount_calls(&self) -> Vec<MountCall> {
            self.calls.lock().expect("mount call log poisoned").clone()
        }

        fn clear_mounts(&self) {
            self.mounted.lock().expect("mount set poisoned").clear();
        }

        fn unmount_calls(&self) -> Vec<PathBuf> {
            self.unmounts.lock().expect("unmount log poisoned").clone()
        }
    }

    impl OverlayMountDriver for TestMountDriver {
        fn mount(
            &self,
            lower_dirs: &[PathBuf],
            upper: &Path,
            work: &Path,
            merged: &Path,
        ) -> Result<(), String> {
            self.calls
                .lock()
                .expect("mount call log poisoned")
                .push(MountCall {
                    lower_dirs: lower_dirs.to_vec(),
                    upper: upper.to_path_buf(),
                    work: work.to_path_buf(),
                    merged: merged.to_path_buf(),
                });
            self.mounted
                .lock()
                .expect("mount set poisoned")
                .insert(merged.to_path_buf());
            Ok(())
        }

        fn unmount(&self, merged: &Path) -> Result<(), String> {
            self.unmounts
                .lock()
                .expect("unmount log poisoned")
                .push(merged.to_path_buf());
            self.mounted
                .lock()
                .expect("mount set poisoned")
                .remove(merged);
            Ok(())
        }

        fn is_mounted(&self, merged: &Path) -> Result<bool, String> {
            Ok(self
                .mounted
                .lock()
                .expect("mount set poisoned")
                .contains(merged))
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

    impl OverlayRepoSource for TestRepoSource {
        fn resolve_repo(
            &self,
            repo_id: &RepoId,
            starting_ref: Option<&str>,
        ) -> Result<ResolvedOverlayBase, WorkspaceError> {
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
            Ok(ResolvedOverlayBase {
                lower_dir: path,
                base_revision,
                branch: starting_ref.map(|value| value.to_string()),
            })
        }
    }

    fn unique_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cairn-overlay-{label}-{}-{}",
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
            strategy: SandboxStrategyRequest::Force(SandboxStrategy::Overlay),
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
            strategy: SandboxStrategyRequest::Force(SandboxStrategy::Overlay),
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

    fn provider_with(
        base_dir: PathBuf,
        mount_driver: Arc<TestMountDriver>,
        repo_source: Arc<TestRepoSource>,
    ) -> OverlayProvider {
        OverlayProvider::with_dependencies(base_dir, mount_driver, repo_source)
    }

    fn create_repo_dir(root: &Path, head: &str) -> PathBuf {
        let repo = root.join("repo");
        fs::create_dir_all(repo.join(".git")).expect("repo git dir should be creatable");
        fs::write(repo.join(".git").join("HEAD"), format!("{head}\n"))
            .expect("repo head should be writable");
        repo
    }

    fn write_overlay_metadata(root: &Path, run_id: &RunId, strategy: SandboxStrategy) {
        let metadata = SandboxMetadata {
            sandbox_id: crate::sandbox::SandboxId::new(format!("sbx-{}", run_id.as_str())),
            run_id: run_id.clone(),
            task_id: None,
            project: cairn_domain::ProjectKey::new("tenant-a", "workspace-a", "project-a"),
            strategy,
            state: SandboxState::Ready,
            base_rev: None,
            repo_id: None,
            path: root.join("merged"),
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
    async fn provision_creates_overlay_layout_for_directory_base() {
        let base_dir = unique_test_dir("provision");
        let source_dir = base_dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("README.md"), "hello").unwrap();

        let mount_driver = Arc::new(TestMountDriver::default());
        let repo_source = Arc::new(TestRepoSource::default());
        let provider = provider_with(base_dir.clone(), mount_driver.clone(), repo_source);
        let run = run_id("run-provision");

        let sandbox = provider
            .provision(&run, &directory_policy(source_dir.clone()))
            .await
            .unwrap();

        let root = base_dir.join("sbx-run-provision");
        assert_eq!(sandbox.path, root.join("merged"));
        assert!(root.join("upper").is_dir());
        assert!(root.join("work").is_dir());
        assert!(root.join("merged").is_dir());
        assert!(root.join("overlay-state.json").is_file());
        assert_eq!(
            fs::read_to_string(root.join("strategy")).unwrap(),
            "overlay_fs"
        );

        let calls = mount_driver.mount_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].lower_dirs, vec![source_dir]);
    }

    #[tokio::test]
    async fn checkpoint_rotates_upper_layers_and_remounts() {
        let base_dir = unique_test_dir("checkpoint");
        let source_dir = base_dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();

        let mount_driver = Arc::new(TestMountDriver::default());
        let provider = provider_with(
            base_dir.clone(),
            mount_driver.clone(),
            Arc::new(TestRepoSource::default()),
        );
        let run = run_id("run-checkpoint");

        provider
            .provision(&run, &directory_policy(source_dir.clone()))
            .await
            .unwrap();
        let root = base_dir.join("sbx-run-checkpoint");
        fs::write(root.join("upper").join("draft.txt"), "draft-v1").unwrap();

        let checkpoint = provider
            .checkpoint(&run, CheckpointKind::Intent)
            .await
            .unwrap();
        assert_eq!(checkpoint.upper_snapshot, Some(root.join("upper.prev.0")));
        assert!(root.join("upper.prev.0").join("draft.txt").is_file());
        assert!(root.join("upper").is_dir());

        let calls = mount_driver.mount_calls();
        assert_eq!(
            calls.last().unwrap().lower_dirs,
            vec![root.join("upper.prev.0"), source_dir]
        );
    }

    #[tokio::test]
    async fn reconnect_remounts_and_uses_previous_upper_layers() {
        let base_dir = unique_test_dir("reconnect");
        let source_dir = base_dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();

        let mount_driver = Arc::new(TestMountDriver::default());
        let provider = provider_with(
            base_dir.clone(),
            mount_driver.clone(),
            Arc::new(TestRepoSource::default()),
        );
        let run = run_id("run-reconnect");

        provider
            .provision(&run, &directory_policy(source_dir.clone()))
            .await
            .unwrap();
        let root = base_dir.join("sbx-run-reconnect");
        fs::write(root.join("upper").join("draft.txt"), "draft-v1").unwrap();
        provider
            .checkpoint(&run, CheckpointKind::Result)
            .await
            .unwrap();

        mount_driver.clear_mounts();
        let sandbox = provider.reconnect(&run).await.unwrap().unwrap();
        assert!(sandbox.is_resumed);

        let calls = mount_driver.mount_calls();
        assert_eq!(
            calls.last().unwrap().lower_dirs,
            vec![root.join("upper.prev.0"), source_dir]
        );
    }

    #[tokio::test]
    async fn reconnect_detects_base_revision_drift_for_repo_base() {
        let base_dir = unique_test_dir("drift");
        let repo_dir = create_repo_dir(&base_dir, "abc123");
        let repo_id = RepoId::new("octocat/hello");

        let mount_driver = Arc::new(TestMountDriver::default());
        let repo_source = Arc::new(TestRepoSource::default());
        repo_source.insert_repo(repo_id.clone(), repo_dir.clone());
        let provider = provider_with(base_dir.clone(), mount_driver, repo_source.clone());
        let run = run_id("run-drift");

        provider
            .provision(&run, &repo_policy(repo_id.clone(), Some("main")))
            .await
            .unwrap();
        fs::write(repo_dir.join(".git").join("HEAD"), "def456\n").unwrap();

        let error = provider.reconnect(&run).await.unwrap_err();
        assert!(matches!(
            error,
            WorkspaceError::BaseRevisionDrift {
                expected,
                actual,
                ..
            } if expected == "abc123" && actual == "def456"
        ));
    }

    #[tokio::test]
    async fn destroy_counts_mutations_and_removes_sandbox_root() {
        let base_dir = unique_test_dir("destroy");
        let source_dir = base_dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();

        let mount_driver = Arc::new(TestMountDriver::default());
        let provider = provider_with(
            base_dir.clone(),
            mount_driver.clone(),
            Arc::new(TestRepoSource::default()),
        );
        let run = run_id("run-destroy");

        provider
            .provision(&run, &directory_policy(source_dir))
            .await
            .unwrap();
        let root = base_dir.join("sbx-run-destroy");
        fs::write(root.join("upper").join("draft.txt"), "draft-v1").unwrap();
        provider
            .checkpoint(&run, CheckpointKind::Intent)
            .await
            .unwrap();
        fs::write(root.join("upper").join("draft-2.txt"), "draft-v2").unwrap();

        let result = provider.destroy(&run, false).await.unwrap();
        assert_eq!(result.files_changed, 2);
        assert_eq!(result.bytes_written, 16);
        assert!(!root.exists());
        assert_eq!(mount_driver.unmount_calls().len(), 2);
    }

    #[tokio::test]
    async fn list_uses_persisted_metadata_for_overlay_sandboxes() {
        let base_dir = unique_test_dir("list");
        let overlay_root = base_dir.join("sbx-run-list");
        fs::create_dir_all(&overlay_root).unwrap();
        write_overlay_metadata(&overlay_root, &run_id("run-list"), SandboxStrategy::Overlay);

        let reflink_root = base_dir.join("sbx-run-reflink");
        fs::create_dir_all(&reflink_root).unwrap();
        write_overlay_metadata(
            &reflink_root,
            &run_id("run-reflink"),
            SandboxStrategy::Reflink,
        );

        let provider = provider_with(
            base_dir,
            Arc::new(TestMountDriver::default()),
            Arc::new(TestRepoSource::default()),
        );

        let handles = provider.list().await.unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].metadata.run_id, run_id("run-list"));
    }
}
