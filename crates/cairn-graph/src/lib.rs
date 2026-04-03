//! Graph projections and graph-backed product query boundaries.
//!
//! `cairn-graph` owns the graph layer for provenance, execution, and
//! knowledge relationships (RFC 004):
//!
//! - **Projections**: typed nodes and edges built from runtime events
//! - **Queries**: product-shaped graph queries for the v1 query families
//! - **Provenance**: execution and retrieval provenance chains

pub mod eval_projector;
pub mod event_projector;
pub mod graph_provenance;
#[cfg(feature = "postgres")]
pub mod pg;
pub mod projections;
pub mod provenance;
pub mod queries;
pub mod retrieval_projector;

pub use projections::{EdgeKind, GraphEdge, GraphNode, GraphProjection, NodeKind};
pub use provenance::{ProvenanceError, ProvenanceService};
pub use queries::{GraphQuery, GraphQueryError, GraphQueryService, TraversalDirection};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles_with_domain_dependency() {
        let id = cairn_domain::SessionId::new("sess_1");
        assert_eq!(id.as_str(), "sess_1");
    }
}
