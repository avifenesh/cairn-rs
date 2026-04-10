use std::collections::HashMap;
use std::path::PathBuf;

use cairn_domain::{CheckpointKind, RunId};

use crate::sandbox::{SandboxBase, SandboxStrategy};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SandboxId(String);

impl SandboxId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvisionedSandbox {
    pub sandbox_id: SandboxId,
    pub run_id: RunId,
    pub path: PathBuf,
    pub base: SandboxBase,
    pub strategy: SandboxStrategy,
    pub branch: Option<String>,
    pub is_resumed: bool,
    pub env: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxCheckpoint {
    pub sandbox_id: SandboxId,
    pub kind: CheckpointKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestroyResult {
    pub sandbox_id: SandboxId,
}
