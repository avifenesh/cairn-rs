use async_trait::async_trait;
use cairn_graph::queries::Subgraph;
use serde::{Deserialize, Serialize};

/// Graph query request for operator visualization.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphQueryRequest {
    pub root_node_id: String,
    pub max_depth: Option<u32>,
}

/// Graph visualization endpoints per RFC 010.
#[async_trait]
pub trait GraphEndpoints: Send + Sync {
    type Error;
    async fn execution_trace(&self, request: &GraphQueryRequest) -> Result<Subgraph, Self::Error>;
    async fn retrieval_provenance(&self, node_id: &str) -> Result<Subgraph, Self::Error>;
}
