//! Trace and provenance handlers.
//!
//! Extracted from `lib.rs` — contains execution trace, retrieval provenance,
//! get trace, prompt provenance, dependency path, graph provenance,
//! multi-hop graph, and graph query response endpoints.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_domain::PromptReleaseId;
use cairn_graph::graph_provenance::GraphProvenanceService;
use cairn_graph::in_memory::InMemoryGraphStore;
use cairn_graph::projections::{GraphNode, NodeKind};
use cairn_graph::provenance::ProvenanceService;
use cairn_graph::{GraphQuery, GraphQueryService, TraversalDirection};
use cairn_store::EventLog;

use crate::errors::AppApiError;
use crate::helpers::event_type_name;
use crate::state::AppState;

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct GraphDepthQuery {
    pub max_depth: Option<u32>,
}

/// RFC 011: a single span within a distributed trace.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct TraceSpan {
    pub event_type: String,
    pub entity_id: Option<String>,
    pub timestamp_ms: u64,
    pub description: String,
}

/// RFC 011: the full trace view returned by GET /v1/trace/:trace_id.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct TraceView {
    pub trace_id: String,
    pub spans: Vec<TraceSpan>,
}

/// Query params for GET /v1/graph/multi-hop/:node_id
#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct MultiHopQuery {
    pub max_hops: Option<u32>,
    /// Minimum edge confidence [0.0, 1.0]. Edges below this threshold are pruned.
    pub min_confidence: Option<f64>,
    /// "upstream" or "downstream" (default: downstream)
    pub direction: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct GraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<cairn_graph::projections::GraphEdge>,
}

impl From<cairn_graph::queries::Subgraph> for GraphResponse {
    fn from(value: cairn_graph::queries::Subgraph) -> Self {
        Self {
            nodes: value.nodes,
            edges: value.edges,
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

pub(crate) async fn graph_query_response(
    graph: &InMemoryGraphStore,
    query: GraphQuery,
) -> axum::response::Response {
    match graph.query(query).await {
        Ok(subgraph) => (StatusCode::OK, Json(GraphResponse::from(subgraph))).into_response(),
        Err(err) => {
            tracing::error!("graph query failed: {err}");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response()
        }
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

pub(crate) async fn execution_trace_handler(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Query(query): Query<GraphDepthQuery>,
) -> impl IntoResponse {
    graph_query_response(
        state.graph.as_ref(),
        GraphQuery::ExecutionTrace {
            root_node_id: run_id,
            root_kind: NodeKind::Run,
            max_depth: query.max_depth.unwrap_or(5),
        },
    )
    .await
}

pub(crate) async fn retrieval_provenance_handler(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    graph_query_response(
        state.graph.as_ref(),
        GraphQuery::RetrievalProvenance {
            answer_node_id: run_id,
        },
    )
    .await
}

pub(crate) async fn get_trace_handler(
    State(state): State<Arc<AppState>>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    let events = match state.runtime.store.read_stream(None, usize::MAX).await {
        Ok(events) => events,
        Err(err) => {
            tracing::error!("trace read_stream failed: {err}");
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response();
        }
    };

    let spans: Vec<TraceSpan> = events
        .into_iter()
        .filter(|stored| {
            stored
                .envelope
                .correlation_id
                .as_deref()
                .map(|t| t == trace_id)
                .unwrap_or(false)
        })
        .map(|stored| {
            let event_type = event_type_name(&stored.envelope.payload).to_owned();
            let entity_id = stored
                .envelope
                .primary_entity_ref()
                .map(|r| format!("{r:?}"));
            let description = format!("{} at position {}", event_type, stored.position.0);
            TraceSpan {
                event_type,
                entity_id,
                timestamp_ms: stored.stored_at,
                description,
            }
        })
        .collect();

    (StatusCode::OK, Json(TraceView { trace_id, spans })).into_response()
}

pub(crate) async fn prompt_provenance_handler(
    State(state): State<Arc<AppState>>,
    Path(release_id): Path<String>,
) -> impl IntoResponse {
    graph_query_response(
        state.graph.as_ref(),
        GraphQuery::PromptProvenance {
            outcome_node_id: PromptReleaseId::new(release_id).to_string(),
        },
    )
    .await
}

pub(crate) async fn dependency_path_handler(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Query(query): Query<GraphDepthQuery>,
) -> impl IntoResponse {
    graph_query_response(
        state.graph.as_ref(),
        GraphQuery::DependencyPath {
            node_id: run_id,
            direction: TraversalDirection::Downstream,
            max_depth: query.max_depth.unwrap_or(5),
        },
    )
    .await
}

pub(crate) async fn graph_provenance_handler(
    State(state): State<Arc<AppState>>,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    let provenance = GraphProvenanceService::new(state.graph.clone());
    match provenance.provenance_chain(&node_id, 10).await {
        Ok(chain) => {
            let nodes = state.graph.all_nodes();
            let mut path = Vec::new();
            if let Some(root) = nodes.get(&node_id).cloned() {
                path.push(root);
            }
            for link in chain.links {
                if let Some(node) = nodes.get(&link.node_id).cloned() {
                    path.push(node);
                }
            }
            (StatusCode::OK, Json(path)).into_response()
        }
        Err(err) => AppApiError::new(
            StatusCode::BAD_REQUEST,
            "provenance_failed",
            err.to_string(),
        )
        .into_response(),
    }
}

/// `GET /v1/graph/multi-hop/:node_id` — generic BFS traversal from a node.
///
/// Query params:
/// - `max_hops` — how many hops to walk (default: 4)
/// - `min_confidence` — prune edges whose confidence is below this value
/// - `direction` — `upstream` or `downstream` (default: `downstream`)
pub(crate) async fn multi_hop_graph_handler(
    State(state): State<Arc<AppState>>,
    Path(node_id): Path<String>,
    Query(query): Query<MultiHopQuery>,
) -> impl IntoResponse {
    let direction = match query.direction.as_deref() {
        Some("upstream") => TraversalDirection::Upstream,
        _ => TraversalDirection::Downstream,
    };
    graph_query_response(
        state.graph.as_ref(),
        GraphQuery::MultiHop {
            start_node_id: node_id,
            max_hops: query.max_hops.unwrap_or(4),
            min_confidence: query.min_confidence,
            direction,
        },
    )
    .await
}
