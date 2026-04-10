use std::path::PathBuf;

use cairn_domain::{
    CheckpointKind, DestroyReason, PreservationReason, ProjectKey, ResourceDimension, RunId, TaskId,
};

use crate::sandbox::{RepoId, SandboxId, SandboxPolicy, SandboxStrategy};

pub type SandboxPolicySnapshot = SandboxPolicy;
pub type SandboxCheckpointKind = CheckpointKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxErrorKind {
    PolicyViolation,
    ProviderUnavailable,
    Filesystem,
    CredentialResolution,
    Recovery,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxEvent {
    SandboxProvisioned {
        sandbox_id: SandboxId,
        run_id: RunId,
        task_id: Option<TaskId>,
        project: ProjectKey,
        strategy: SandboxStrategy,
        base_revision: Option<String>,
        policy: SandboxPolicySnapshot,
        path: PathBuf,
        duration_ms: u64,
        provisioned_at: u64,
    },
    SandboxActivated {
        sandbox_id: SandboxId,
        run_id: RunId,
        pid: Option<u32>,
        activated_at: u64,
    },
    SandboxHeartbeat {
        sandbox_id: SandboxId,
        run_id: RunId,
        heartbeat_at: u64,
    },
    SandboxCheckpointed {
        sandbox_id: SandboxId,
        run_id: RunId,
        checkpoint_kind: SandboxCheckpointKind,
        rescue_ref: Option<String>,
        upper_snapshot: Option<PathBuf>,
        checkpointed_at: u64,
    },
    SandboxPreserved {
        sandbox_id: SandboxId,
        run_id: RunId,
        reason: PreservationReason,
        preserved_at: u64,
    },
    SandboxDestroyed {
        sandbox_id: SandboxId,
        run_id: RunId,
        files_changed: u32,
        bytes_written: u64,
        reason: DestroyReason,
        destroyed_at: u64,
    },
    SandboxProvisioningFailed {
        sandbox_id: SandboxId,
        run_id: RunId,
        error_kind: SandboxErrorKind,
        error: String,
        failed_at: u64,
    },
    SandboxPolicyDegraded {
        sandbox_id: SandboxId,
        run_id: RunId,
        requested: SandboxStrategy,
        actual: SandboxStrategy,
        reason: String,
        degraded_at: u64,
    },
    SandboxResourceLimitExceeded {
        sandbox_id: SandboxId,
        run_id: RunId,
        dimension: ResourceDimension,
        limit: u64,
        observed: u64,
        at: u64,
    },
    SandboxBaseRevisionDrift {
        sandbox_id: SandboxId,
        run_id: RunId,
        project: ProjectKey,
        repo_id: RepoId,
        expected: String,
        actual: String,
        detected_at: u64,
    },
    SandboxAllowlistRevoked {
        sandbox_id: SandboxId,
        run_id: RunId,
        project: ProjectKey,
        repo_id: RepoId,
        revoked_at: u64,
        detected_at: u64,
    },
}
