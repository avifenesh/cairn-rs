//! Durable runtime services for sessions, runs, tasks, approvals, and recovery.
//!
//! `cairn-runtime` owns the runtime service boundaries that accept
//! commands, validate state transitions, persist events, and update
//! synchronous projections through `cairn-store`.

pub mod approval_policies;
pub mod approvals;
pub mod checkpoints;
pub mod enrichment;
pub mod error;
pub mod eval_runs;
pub mod ingest_jobs;
pub mod mailbox;
pub mod projects;
pub mod prompt_assets;
pub mod prompt_releases;
pub mod prompt_versions;
pub mod recovery;
pub mod routing;
pub mod runs;
pub mod services;
pub mod sessions;
pub mod signals;
pub mod tasks;
pub mod tenants;
pub mod workspaces;

pub use approval_policies::ApprovalPolicyService;
pub use approvals::ApprovalService;
pub use checkpoints::CheckpointService;
pub use enrichment::{
    ApprovalEnrichment, CheckpointEnrichment, RunEnrichment, RuntimeEnrichment, SessionEnrichment,
    StoreBackedEnrichment, TaskEnrichment,
};
pub use error::RuntimeError;
pub use mailbox::MailboxService;
pub use recovery::{RecoveryAction, RecoveryService, RecoverySummary};
pub use runs::RunService;
pub use eval_runs::EvalRunService;
pub use ingest_jobs::IngestJobService;
pub use prompt_assets::PromptAssetService;
pub use prompt_releases::PromptReleaseService;
pub use prompt_versions::PromptVersionService;
pub use services::{
    ApprovalPolicyServiceImpl, ApprovalServiceImpl, CheckpointServiceImpl, EvalRunServiceImpl, ExternalWorkerService,
    ExternalWorkerServiceImpl, IngestJobServiceImpl, MailboxServiceImpl,
    ProjectServiceImpl, PromptAssetServiceImpl, PromptReleaseServiceImpl,
    PromptVersionServiceImpl, RecoveryServiceImpl,
    RunServiceImpl, SessionServiceImpl, SignalServiceImpl, TaskServiceImpl,
    TenantServiceImpl, ToolInvocationService, ToolInvocationServiceImpl,
    SimpleRouteResolver, WorkspaceServiceImpl,
};
pub use projects::ProjectService;
pub use routing::RouteResolverService;
pub use sessions::SessionService;
pub use signals::SignalService;
pub use tasks::TaskService;
pub use tenants::TenantService;
pub use workspaces::WorkspaceService;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles_with_domain_and_store_deps() {
        let id = cairn_domain::SessionId::new("test");
        assert_eq!(id.as_str(), "test");
    }
}
