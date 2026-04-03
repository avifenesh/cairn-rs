//! Product-owned ingest, retrieval, and memory service boundaries.
//!
//! `cairn-memory` owns the retrieval pipeline that replaces Bedrock KB (RFC 003):
//!
//! - **Ingest**: source registration, parsing, chunking, embedding, indexing
//! - **Retrieval**: lexical, vector, and hybrid query with inspectable scoring
//! - **Diagnostics**: source quality, index status, operator visibility
//! - **Deep search**: multi-hop iterative retrieval with quality gates

pub mod api_impl;
pub mod bundles;
pub mod deep_search;
pub mod deep_search_impl;
pub mod diagnostics;
pub mod diagnostics_impl;
pub mod feed_impl;
pub mod graph_ingest;
pub mod in_memory;
pub mod ingest;
#[cfg(feature = "postgres")]
pub mod pg;
pub mod pipeline;
pub mod reranking;
pub mod retrieval;
pub mod services;
#[cfg(feature = "sqlite")]
pub mod sqlite;

pub use deep_search::{DeepSearchError, DeepSearchService};
pub use diagnostics::{DiagnosticsError, DiagnosticsService};
pub use ingest::{IngestError, IngestService, IngestStatus, SourceType};
pub use retrieval::{RetrievalError, RetrievalMode, RetrievalService};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles_with_domain_dependency() {
        let id = cairn_domain::KnowledgeDocumentId::new("doc_1");
        assert_eq!(id.as_str(), "doc_1");
    }
}
