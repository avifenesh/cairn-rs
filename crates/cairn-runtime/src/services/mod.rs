//! Concrete runtime service implementations.
//!
//! Each service accepts command parameters, validates state transitions,
//! emits events through the EventLog, and returns the updated projection.

pub mod approval_impl;
pub mod approval_policy_impl;
pub mod checkpoint_impl;
pub mod eval_run_impl;
pub mod event_helpers;
pub mod external_worker_impl;
pub mod ingest_job_impl;
pub mod mailbox_impl;
pub mod project_impl;
pub mod prompt_asset_impl;
pub mod prompt_release_impl;
pub mod prompt_version_impl;
pub mod recovery_impl;
pub mod route_resolver_impl;
pub mod run_impl;
pub mod session_impl;
pub mod signal_impl;
pub mod task_impl;
pub mod tenant_impl;
pub mod tool_invocation_impl;
pub mod workspace_impl;

pub use approval_impl::ApprovalServiceImpl;
pub use approval_policy_impl::ApprovalPolicyServiceImpl;
pub use checkpoint_impl::CheckpointServiceImpl;
pub use eval_run_impl::EvalRunServiceImpl;
pub use external_worker_impl::{parse_outcome, ExternalWorkerService, ExternalWorkerServiceImpl};
pub use ingest_job_impl::IngestJobServiceImpl;
pub use mailbox_impl::MailboxServiceImpl;
pub use project_impl::ProjectServiceImpl;
pub use prompt_asset_impl::PromptAssetServiceImpl;
pub use prompt_release_impl::PromptReleaseServiceImpl;
pub use prompt_version_impl::PromptVersionServiceImpl;
pub use recovery_impl::RecoveryServiceImpl;
pub use route_resolver_impl::SimpleRouteResolver;
pub use run_impl::RunServiceImpl;
pub use session_impl::SessionServiceImpl;
pub use signal_impl::SignalServiceImpl;
pub use task_impl::TaskServiceImpl;
pub use tenant_impl::TenantServiceImpl;
pub use tool_invocation_impl::{ToolInvocationService, ToolInvocationServiceImpl};
pub use workspace_impl::WorkspaceServiceImpl;
