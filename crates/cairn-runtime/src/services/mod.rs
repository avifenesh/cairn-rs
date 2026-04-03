//! Concrete runtime service implementations.
//!
//! Each service accepts command parameters, validates state transitions,
//! emits events through the EventLog, and returns the updated projection.

pub mod approval_impl;
pub mod checkpoint_impl;
pub mod event_helpers;
pub mod external_worker_impl;
pub mod ingest_job_impl;
pub mod mailbox_impl;
pub mod recovery_impl;
pub mod run_impl;
pub mod session_impl;
pub mod signal_impl;
pub mod task_impl;
pub mod tool_invocation_impl;

pub use approval_impl::ApprovalServiceImpl;
pub use checkpoint_impl::CheckpointServiceImpl;
pub use external_worker_impl::{parse_outcome, ExternalWorkerService, ExternalWorkerServiceImpl};
pub use ingest_job_impl::IngestJobServiceImpl;
pub use mailbox_impl::MailboxServiceImpl;
pub use recovery_impl::RecoveryServiceImpl;
pub use run_impl::RunServiceImpl;
pub use session_impl::SessionServiceImpl;
pub use signal_impl::SignalServiceImpl;
pub use task_impl::TaskServiceImpl;
pub use tool_invocation_impl::{ToolInvocationService, ToolInvocationServiceImpl};
