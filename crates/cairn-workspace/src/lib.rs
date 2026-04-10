//! Sandbox workspace primitives for RFC 016.

pub mod error;
pub mod providers;
pub mod repo_store;
pub mod sandbox;

pub use error::{RepoStoreError, SweepError, WorkspaceError};
pub use providers::{OverlayProvider, ReflinkProvider, SandboxProvider};
pub use repo_store::{
    ActiveSandboxRepoSource, ProjectRepoAccessService, RefreshOutcome, RepoCloneCache,
    RepoCloneSweepTask, RepoStore, RepoStoreEvent, SweepId,
};
pub use sandbox::{
    CredentialReference, DestroyResult, HostCapabilityRequirements, ProvisionedSandbox, RepoId,
    SandboxBase, SandboxCheckpoint, SandboxId, SandboxMetadata, SandboxPolicy, SandboxService,
    SandboxState, SandboxStrategy, SandboxStrategyRequest,
};

#[cfg(test)]
mod tests {
    use super::RepoId;

    #[test]
    fn repo_id_preserves_owner_repo_shape() {
        assert_eq!(RepoId::new("octocat/hello").as_str(), "octocat/hello");
    }
}
