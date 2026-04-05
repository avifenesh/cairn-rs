//! Shared domain contracts for Cairn runtime, storage, and product services.

pub mod agent_roles;
pub mod audit;
pub mod bandit;
pub mod channel;
pub mod checkpoint_strategy;
pub mod commercial;
pub mod commands;
pub mod compaction;
pub mod credentials;
pub mod defaults;
pub mod errors;
pub mod evals;
pub mod events;
pub mod ids;
pub mod ingest_job;
pub mod lifecycle;
pub mod model_catalog;
pub mod notification_prefs;
pub mod observability;
pub mod onboarding;
pub mod org;
pub mod policy;
pub mod prompts;
pub mod providers;
pub mod quotas;
pub mod recovery;
pub mod research;
pub mod resource_sharing;
pub mod selectors;
pub mod signal;
pub mod skills;
pub mod sla;
pub mod soul;
pub mod task_dependencies;
pub mod tenancy;
pub mod tool_invocation;
pub mod voice;
pub mod workers;

pub use agent_roles::*;
pub use audit::*;
pub use channel::*;
pub use checkpoint_strategy::*;
pub use commercial::*;
pub use compaction::*;
pub use commands::*;
pub use credentials::*;
pub use defaults::*;
pub use errors::*;
pub use evals::*;
pub use events::*;
pub use ids::*;
pub use ingest_job::*;
pub use lifecycle::*;
pub use notification_prefs::*;
pub use observability::LlmCallTrace;
pub use onboarding::*;
pub use org::*;
pub use policy::*;
pub use prompts::*;
pub use providers::*;
pub use quotas::*;
pub use recovery::*;
pub use research::*;
pub use resource_sharing::*;
pub use selectors::*;
pub use signal::*;
pub use skills::*;
pub use sla::*;
pub use task_dependencies::TaskDependency;
pub use tenancy::*;
pub use tool_invocation::*;
pub use voice::{
    SpeechToTextRequest, SpeechToTextResult, TextToSpeechRequest, TextToSpeechResult,
    TranscriptSegment, VoiceFormat,
};
pub use workers::*;

/// Feature flag constants used by the entitlement gate (cairn-app references these by name).
pub const CREDENTIAL_MANAGEMENT: &str = "credential_management";
pub const EVAL_MATRICES: &str = "eval_matrices";
pub const MULTI_PROVIDER: &str = "multi_provider";
