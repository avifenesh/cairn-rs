use std::path::PathBuf;

use cairn_domain::{ProjectKey, RunId};

use crate::sandbox::{RepoId, SandboxId, SandboxState, SandboxStrategy};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxMetadata {
    pub sandbox_id: SandboxId,
    pub run_id: RunId,
    pub project: ProjectKey,
    pub strategy: SandboxStrategy,
    pub state: SandboxState,
    pub base_rev: Option<String>,
    pub repo_id: Option<RepoId>,
    pub path: PathBuf,
    pub pid: Option<u32>,
    pub created_at: u64,
    pub heartbeat_at: u64,
    pub policy_hash: String,
}
