pub mod overlay;
pub mod reflink;
pub mod repo_source;

use async_trait::async_trait;
use cairn_domain::{CheckpointKind, ProjectKey, RunId};

use crate::error::WorkspaceError;
use crate::sandbox::{
    DestroyResult, ProvisionedSandbox, SandboxCheckpoint, SandboxHandle, SandboxPolicy,
    SandboxStrategy,
};

pub use overlay::OverlayProvider;
pub use reflink::ReflinkProvider;
pub use repo_source::RepoCloneCacheSource;

#[async_trait]
pub trait SandboxProvider: Send + Sync + 'static {
    fn strategy(&self) -> SandboxStrategy;

    async fn provision(
        &self,
        run_id: &RunId,
        project: &ProjectKey,
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
        project: &ProjectKey,
        policy: &SandboxPolicy,
    ) -> Result<ProvisionedSandbox, WorkspaceError>;

    async fn destroy(
        &self,
        run_id: &RunId,
        preserve: bool,
    ) -> Result<DestroyResult, WorkspaceError>;

    async fn list(&self) -> Result<Vec<SandboxHandle>, WorkspaceError>;

    async fn heartbeat(&self, run_id: &RunId) -> Result<(), WorkspaceError>;
}
