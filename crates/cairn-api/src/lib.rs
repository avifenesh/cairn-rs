//! HTTP, SSE, auth, bootstrap, and operator-facing API boundaries.

pub mod assistant;
pub mod auth;
pub mod bootstrap;
pub mod endpoints;
pub mod evals_api;
pub mod external_workers;
pub mod feed;
pub mod http;
pub mod memory_api;
pub mod operator;
pub mod overview;
pub mod provenance;
pub mod read_models;
pub mod sources_channels;
pub mod sse;
pub mod sse_payloads;
pub mod sse_publisher;

pub use assistant::{
    AssistantEndpoints, AssistantMessageRequest, AssistantMessageResponse, AssistantSession,
    ChatMessage, ChatRole,
};
pub use auth::{AuthPrincipal, Authenticator, Authorizer};
pub use bootstrap::{BootstrapConfig, DeploymentMode, ServerBootstrap};
pub use endpoints::{ListQuery, RuntimeReadEndpoints};
pub use external_workers::{ExternalWorkerEndpoints, WorkerReportRequest};
pub use feed::{FeedEndpoints, FeedItem, FeedQuery};
pub use http::{
    HealthResponse, HttpMethod, ListResponse, OkResponse, RouteClassification, RouteEntry,
    RouteRegistry,
};
pub use memory_api::{
    CreateMemoryRequest, MemoryEndpoints, MemoryItem, MemorySearchQuery, MemoryStatus,
};
pub use operator::{OperatorCommandEndpoints, OperatorReadEndpoints, RunDetail};
pub use overview::{
    CostSummary, DashboardOverview, MetricsSummary, OverviewEndpoints, SystemStatus,
};
pub use read_models::{ApprovalSummary, ReadModelQuery, RunSummary, TaskSummary};
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
