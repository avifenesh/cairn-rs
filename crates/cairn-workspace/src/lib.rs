//! Sandbox workspace primitives for RFC 016.

pub mod error;
pub mod providers;
pub mod repo_store;
pub mod sandbox;

pub use error::{RepoStoreError, SweepError, WorkspaceError};
pub use providers::{OverlayProvider, ReflinkProvider, RepoCloneCacheSource, SandboxProvider};
pub use repo_store::{
    ActiveSandboxRepoSource, ProjectRepoAccessService, RefreshOutcome, RepoCloneCache,
    RepoCloneSweepTask, RepoStore, RepoStoreEvent, SweepId,
};
pub use sandbox::{
    BufferedSandboxEventSink, Clock, CredentialReference, DestroyResult,
    HostCapabilityRequirements, InvalidRepoId, ProvisionedSandbox, RepoId, SandboxBase,
    SandboxCheckpoint, SandboxCheckpointKind, SandboxErrorKind, SandboxEvent, SandboxEventSink,
    SandboxHandle, SandboxId, SandboxMetadata, SandboxPolicy, SandboxPolicySnapshot,
    SandboxRecoverySummary, SandboxService, SandboxState, SandboxStrategy, SandboxStrategyRequest,
    SystemClock,
};

#[cfg(test)]
mod tests {
    use super::RepoId;

    #[test]
    fn repo_id_preserves_owner_repo_shape() {
        assert_eq!(
            RepoId::parse("octocat/hello").unwrap().as_str(),
            "octocat/hello"
        );
        assert!(RepoId::parse("../escape").is_err());
    }
}
