//! RFC 003 graph proximity scoring integration tests.
//!
//! Chunks score higher when their source document is graph-linked to other
//! result documents. Implemented as a post-scoring pass that queries
//! immediate neighbors of each result's document_id in the graph.

use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_graph::in_memory::InMemoryGraphStore;
use cairn_graph::projections::{EdgeKind, GraphEdge, GraphNode, GraphProjection, NodeKind};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// Add a document node to the graph with kind=Document.
async fn add_doc_node(graph: &InMemoryGraphStore, doc_id: &str) {
    graph
        .add_node(GraphNode {
            node_id: doc_id.to_owned(),
            kind: NodeKind::Document,
            project: Some(project()),
            created_at: cairn_memory::retrieval::now_ms(),
        })
        .await
        .unwrap();
}

/// Add a directed edge between two document nodes.
async fn add_doc_edge(graph: &InMemoryGraphStore, from: &str, to: &str) {
    graph
        .add_edge(GraphEdge {
            source_node_id: from.to_owned(),
            target_node_id: to.to_owned(),
            kind: EdgeKind::DerivedFrom,
            created_at: cairn_memory::retrieval::now_ms(),
            confidence: None,
        })
        .await
        .unwrap();
}

/// Two documents are connected by a graph edge.
/// A third standalone document has no graph edges.
///
/// After retrieval:
/// - doc_a and doc_b must have graph_proximity > 0 (they see each other as neighbors).
/// - doc_c must have graph_proximity = 0 (isolated in the graph).
/// - doc_a and doc_b must score higher than doc_c (all else equal).
#[tokio::test]
async fn graph_proximity_linked_docs_score_higher_than_isolated() {
    let graph = Arc::new(InMemoryGraphStore::new());
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone()).with_graph(graph.clone());

    // Ingest three documents with unique but query-matching content.
    for (doc_id, src, body) in [
        ("doc_a", "src_a", "Rust memory safety ownership borrow checker alpha edition"),
        ("doc_b", "src_b", "Rust memory safety ownership enables fearless concurrency beta"),
        ("doc_c", "src_c", "Rust memory safety ownership prevents dangling references gamma"),
    ] {
        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new(doc_id),
                source_id: SourceId::new(src),
                source_type: SourceType::PlainText,
                project: project(),
                content: body.to_owned(),
                tags: vec![],
                corpus_id: None,
                bundle_source_id: None,
                import_id: None,
            })
            .await
            .unwrap();
    }

    // Add graph nodes for doc_a and doc_b and link them.
    add_doc_node(&graph, "doc_a").await;
    add_doc_node(&graph, "doc_b").await;
    add_doc_edge(&graph, "doc_a", "doc_b").await;
    // doc_c gets no graph node or edges — isolated.

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust memory safety".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 3, "all 3 docs should match");

    let find = |id: &str| {
        response
            .results
            .iter()
            .find(|r| r.chunk.document_id == KnowledgeDocumentId::new(id))
            .unwrap_or_else(|| panic!("missing result for {id}"))
    };

    let ra = find("doc_a");
    let rb = find("doc_b");
    let rc = find("doc_c");

    // doc_a sees doc_b as a downstream neighbor → proximity > 0.
    assert!(
        ra.breakdown.graph_proximity > 0.0,
        "doc_a is connected to doc_b — graph_proximity must be > 0, got {}",
        ra.breakdown.graph_proximity
    );

    // doc_b sees doc_a as an upstream neighbor → proximity > 0.
    assert!(
        rb.breakdown.graph_proximity > 0.0,
        "doc_b is connected to doc_a — graph_proximity must be > 0, got {}",
        rb.breakdown.graph_proximity
    );

    // doc_c has no graph edges — proximity must be 0.
    assert_eq!(
        rc.breakdown.graph_proximity, 0.0,
        "doc_c is isolated — graph_proximity must be 0, got {}",
        rc.breakdown.graph_proximity
    );

    // Connected docs score strictly higher than the isolated doc.
    assert!(
        ra.score > rc.score,
        "doc_a (proximity={:.3}) must outscore doc_c (proximity=0): {:.4} vs {:.4}",
        ra.breakdown.graph_proximity,
        ra.score,
        rc.score
    );
    assert!(
        rb.score > rc.score,
        "doc_b (proximity={:.3}) must outscore doc_c (proximity=0): {:.4} vs {:.4}",
        rb.breakdown.graph_proximity,
        rb.score,
        rc.score
    );
}

/// Graph proximity is 0 for all results when no graph service is wired in.
#[tokio::test]
async fn graph_proximity_zero_without_graph_service() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    // No .with_graph() call — graph is None.
    let retrieval = InMemoryRetrieval::new(store.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_no_graph"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "knowledge graph proximity scoring test".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "graph proximity".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(!response.results.is_empty());
    for result in &response.results {
        assert_eq!(
            result.breakdown.graph_proximity, 0.0,
            "no graph service means proximity must be 0"
        );
    }
}

/// A single-result query never gets graph proximity (no other results to connect to).
#[tokio::test]
async fn graph_proximity_single_result_stays_zero() {
    let graph = Arc::new(InMemoryGraphStore::new());
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone()).with_graph(graph.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("solo_doc"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "solo document with unique content about retrieval pipelines".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    add_doc_node(&graph, "solo_doc").await;

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "solo document unique".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(
        response.results[0].breakdown.graph_proximity, 0.0,
        "single result has no peers — proximity must be 0"
    );
}

/// graph_proximity dimension appears in scoring_dimensions_used when active.
#[tokio::test]
async fn graph_proximity_appears_in_scoring_dimensions() {
    let graph = Arc::new(InMemoryGraphStore::new());
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone()).with_graph(graph.clone());

    for (doc_id, src, body) in [
        ("dim_a", "src_dim_a", "scoring dimensions graph proximity test alpha content"),
        ("dim_b", "src_dim_b", "scoring dimensions graph proximity test beta content"),
    ] {
        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new(doc_id),
                source_id: SourceId::new(src),
                source_type: SourceType::PlainText,
                project: project(),
                content: body.to_owned(),
                tags: vec![],
                corpus_id: None,
                bundle_source_id: None,
                import_id: None,
            })
            .await
            .unwrap();
    }

    add_doc_node(&graph, "dim_a").await;
    add_doc_node(&graph, "dim_b").await;
    add_doc_edge(&graph, "dim_a", "dim_b").await;

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "scoring graph proximity".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(
        response
            .diagnostics
            .scoring_dimensions_used
            .contains(&"graph_proximity".to_owned()),
        "graph_proximity must appear in scoring_dimensions_used when active"
    );
}
