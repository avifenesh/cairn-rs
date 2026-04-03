//! Provenance and graph-backed API endpoints.
//!
//! Exposes Worker 6's graph query and provenance services through
//! the API boundary for operator views.

use async_trait::async_trait;
use cairn_graph::queries::Subgraph;
use serde::{Deserialize, Serialize};

/// API request for an execution trace query.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionTraceRequest {
    pub root_node_id: String,
    pub root_kind: String,
    pub max_depth: Option<u32>,
}

/// API request for a retrieval provenance query.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalProvenanceRequest {
    pub answer_node_id: String,
}

/// API-facing provenance endpoint boundary.
#[async_trait]
pub trait ProvenanceEndpoints: Send + Sync {
    type Error;

    /// Query execution trace for a session/run/task.
    async fn execution_trace(
        &self,
        request: &ExecutionTraceRequest,
    ) -> Result<Subgraph, Self::Error>;

    /// Query retrieval provenance: answer -> chunk -> document -> source.
    async fn retrieval_provenance(
        &self,
        request: &RetrievalProvenanceRequest,
    ) -> Result<Subgraph, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_trace_request_serialization() {
        let req = ExecutionTraceRequest {
            root_node_id: "run_1".to_owned(),
            root_kind: "run".to_owned(),
            max_depth: Some(5),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["rootNodeId"], "run_1");
        assert_eq!(json["maxDepth"], 5);
    }

    #[test]
    fn retrieval_provenance_request_serialization() {
        let req = RetrievalProvenanceRequest {
            answer_node_id: "answer_1".to_owned(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["answerNodeId"], "answer_1");
    }
}
