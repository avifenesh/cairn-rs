//! HTTP, SSE, auth, bootstrap, and operator-facing API boundaries.

pub mod admin;
pub mod assistant;
pub mod auth;
pub mod bootstrap;
pub mod endpoints;
pub mod evals_api;
pub mod external_workers;
pub mod feed;
pub mod graph_api;
pub mod http;
pub mod memory_api;
pub mod onboarding;
pub mod operator;
pub mod overview;
pub mod policies_api;
pub mod prompts_api;
pub mod provenance;
pub mod providers_api;
pub mod read_models;
pub mod settings_api;
pub mod sources_channels;
pub mod sse;
pub mod sse_payloads;
pub mod sse_publisher;

pub use admin::AdminEndpoints;
pub use assistant::{
    AssistantEndpoints, AssistantMessageRequest, AssistantMessageResponse, AssistantSession,
    ChatMessage, ChatRole,
};
pub use auth::{AuthPrincipal, Authenticator, Authorizer};
pub use bootstrap::{
    BootstrapConfig, DeploymentMode, EncryptionKeySource, ServerBootstrap, ServerRole,
    StorageBackend,
};
pub use endpoints::{ListQuery, RuntimeReadEndpoints};
pub use external_workers::{ExternalWorkerEndpoints, WorkerReportRequest};
pub use feed::{FeedEndpoints, FeedItem, FeedQuery};
pub use graph_api::{GraphEndpoints, GraphQueryRequest};
pub use http::{
    HealthResponse, HttpMethod, ListResponse, OkResponse, RouteClassification, RouteEntry,
    RouteRegistry,
};
pub use memory_api::{
    CreateMemoryRequest, MemoryEndpoints, MemoryItem, MemorySearchQuery, MemoryStatus,
};
pub use operator::{OperatorCommandEndpoints, OperatorReadEndpoints, RunDetail};
pub use policies_api::{PolicyDecisionSummary, PolicyEndpoints};
pub use prompts_api::PromptEndpoints;
pub use providers_api::{ProviderEndpoints, ProviderHealthSummary};
pub use overview::{
    CostSummary, DashboardOverview, MetricsSummary, OverviewEndpoints, SystemStatus,
};
pub use read_models::{ApprovalSummary, ReadModelQuery, RunSummary, TaskSummary};
pub use settings_api::{SettingsEndpoints, SettingsSummary};
pub use sources_channels::{ChannelEndpoints, SourceEndpoints};
pub use sse::{SseEventEntry, SseEventName, SseFrame, SseStream};
pub use sse_publisher::{SsePublisher, SseReplayQuery};

#[cfg(test)]
mod tests {
    use crate::http::preserved_route_catalog;
    use crate::sse::preserved_sse_catalog;

    #[test]
    fn route_and_sse_catalogs_are_consistent() {
        let routes = preserved_route_catalog();
        let sse = preserved_sse_catalog();

        // Stream endpoint exists in route catalog
        assert!(routes.iter().any(|r| r.path == "/v1/stream"));
        // SSE catalog has the ready event
        assert!(sse.iter().any(|e| e.name == "ready"));
    }
}
