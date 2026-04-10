pub mod overlay;
pub mod reflink;

use async_trait::async_trait;
use cairn_domain::{CheckpointKind, RunId};

use crate::error::WorkspaceError;
use crate::sandbox::{
    DestroyResult, ProvisionedSandbox, SandboxCheckpoint, SandboxPolicy, SandboxStrategy,
};

pub use overlay::OverlayProvider;
pub use reflink::ReflinkProvider;

#[async_trait]
pub trait SandboxProvider: Send + Sync {
    fn strategy(&self) -> SandboxStrategy;

    async fn provision(
        &self,
        run_id: &RunId,
        policy: &SandboxPolicy,
    ) -> Result<ProvisionedSandbox, WorkspaceError>;

    async fn reconnect(&self, run_id: &RunId)
        -> Result<Option<ProvisionedSandbox>, WorkspaceError>;

    async fn checkpoint(
        &self,
        run_id: &RunId,
        kind: CheckpointKind,
    ) -> Result<SandboxCheckpoint, WorkspaceError>;

    async fn restore(
        &self,
        from_checkpoint: &SandboxCheckpoint,
        new_run_id: &RunId,
        policy: &SandboxPolicy,
    ) -> Result<ProvisionedSandbox, WorkspaceError>;

    async fn destroy(
        &self,
        run_id: &RunId,
        preserve: bool,
    ) -> Result<DestroyResult, WorkspaceError>;

    async fn heartbeat(&self, run_id: &RunId) -> Result<(), WorkspaceError>;
}
