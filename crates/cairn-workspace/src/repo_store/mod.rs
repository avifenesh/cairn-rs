pub mod access_service;
pub mod clone_cache;
pub mod facade;
pub mod sweep;

use std::path::PathBuf;

use cairn_domain::{ActorRef, ProjectKey, TenantId};

use crate::sandbox::RepoId;

pub use access_service::ProjectRepoAccessService;
pub use clone_cache::{RefreshOutcome, RepoCloneCache};
pub use facade::RepoStore;
pub use sweep::{ActiveSandboxRepoSource, RepoCloneSweepTask};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SweepId(String);

impl SweepId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoStoreEvent {
    ProjectRepoAllowlistExpanded {
        project: ProjectKey,
        repo_id: RepoId,
        added_by: ActorRef,
        at: u64,
    },
    ProjectRepoAllowlistShrunk {
        project: ProjectKey,
        repo_id: RepoId,
        removed_by: ActorRef,
        at: u64,
    },
    RepoCloneCloning {
        tenant: TenantId,
        repo_id: RepoId,
        started_at: u64,
    },
    RepoCloneCreated {
        tenant: TenantId,
        repo_id: RepoId,
        path: PathBuf,
        duration_ms: u64,
        at: u64,
    },
    RepoCloneFailed {
        tenant: TenantId,
        repo_id: RepoId,
        error: String,
        failed_at: u64,
    },
    RepoCloneLocked {
        tenant: TenantId,
        repo_id: RepoId,
        at: u64,
    },
    RepoCloneDeleted {
        tenant: TenantId,
        repo_id: RepoId,
        sweep_id: Option<SweepId>,
        at: u64,
    },
    RepoStoreRefreshed {
        tenant: TenantId,
        repo_id: RepoId,
        old_head: String,
        new_head: String,
        drifted_sandbox_count: u32,
        at: u64,
    },
    RepoCloneSweepStarted {
        sweep_id: SweepId,
        started_at: u64,
    },
    RepoCloneSweepCompleted {
        sweep_id: SweepId,
        deleted: u32,
        skipped_active_sandboxes: u32,
        skipped_active_allowlists: u32,
        completed_at: u64,
    },
}
