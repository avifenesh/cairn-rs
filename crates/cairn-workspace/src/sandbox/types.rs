use std::collections::HashMap;
use std::path::PathBuf;

use cairn_domain::{CheckpointKind, RunId};
use serde::{Deserialize, Serialize};

use crate::sandbox::{SandboxBase, SandboxMetadata, SandboxStrategy};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SandboxId(String);

impl SandboxId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxState {
    Initial,
    Provisioning,
    Ready,
    Active,
    Checkpointed,
    Preserved,
    Destroying,
    Destroyed,
    Failed,
}

impl SandboxState {
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (SandboxState::Initial, SandboxState::Provisioning)
                | (SandboxState::Provisioning, SandboxState::Ready)
                | (SandboxState::Provisioning, SandboxState::Failed)
                | (SandboxState::Ready, SandboxState::Active)
                | (SandboxState::Ready, SandboxState::Preserved)
                | (SandboxState::Ready, SandboxState::Destroying)
                | (SandboxState::Active, SandboxState::Checkpointed)
                | (SandboxState::Active, SandboxState::Preserved)
                | (SandboxState::Active, SandboxState::Destroying)
                | (SandboxState::Checkpointed, SandboxState::Provisioning)
                | (SandboxState::Checkpointed, SandboxState::Active)
                | (SandboxState::Checkpointed, SandboxState::Preserved)
                | (SandboxState::Checkpointed, SandboxState::Destroying)
                | (SandboxState::Preserved, SandboxState::Provisioning)
                | (SandboxState::Preserved, SandboxState::Active)
                | (SandboxState::Preserved, SandboxState::Destroying)
                | (SandboxState::Destroying, SandboxState::Destroyed)
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, SandboxState::Destroyed | SandboxState::Failed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvisionedSandbox {
    pub sandbox_id: SandboxId,
    pub run_id: RunId,
    pub path: PathBuf,
    pub base: SandboxBase,
    pub strategy: SandboxStrategy,
    pub base_revision: Option<String>,
    pub branch: Option<String>,
    pub is_resumed: bool,
    pub env: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxCheckpoint {
    pub sandbox_id: SandboxId,
    pub run_id: RunId,
    pub kind: CheckpointKind,
    pub rescue_ref: Option<String>,
    pub upper_snapshot: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestroyResult {
    pub sandbox_id: SandboxId,
    pub files_changed: u32,
    pub bytes_written: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxHandle {
    pub metadata: SandboxMetadata,
}
