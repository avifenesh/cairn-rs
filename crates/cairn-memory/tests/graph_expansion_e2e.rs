//! RFC 003/004 deep search graph expansion end-to-end integration test.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_graph::projections::{EdgeKind, GraphEdge, GraphNode, NodeKind};
use cairn_graph::queries::{
    GraphQuery, GraphQueryError, GraphQueryService, Subgraph, TraversalDirection,
};
use cairn_memory::deep_search::{DeepSearchRequest, DeepSearchService};
use cairn_memory::deep_search_impl::{GraphExpansionHook, IterativeDeepSearch, QualityGateConfig};
use cairn_memory::graph_expansion::GraphBackedExpansion;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::ChunkRecord;
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{RetrievalMode, RetrievalResult, ScoringBreakdown};

fn project() -> ProjectKey {
    ProjectKey::new("t_gx", "ws_gx", "proj_gx")
}

fn ingest_req(doc_id: &str, content: &str) -> IngestRequest {
    IngestRequest {
        document_id: KnowledgeDocumentId::new(doc_id),
        source_id: SourceId::new("src_gx"),
        source_type: SourceType::PlainText,
        project: project(),
        content: content.to_owned(),
        import_id: None,
        corpus_id: None,
        bundle_source_id: None,
        tags: vec![],
    }
}

struct TestGraph {
    neighbors: Mutex<HashMap<String, Vec<(GraphEdge, GraphNode)>>>,
}

impl TestGraph {
    fn new() -> Self {
        Self {
            neighbors: Mutex::new(HashMap::new()),
        }
    }

    fn add_link(&self, from_doc: &str, to_doc: &str, edge_kind: EdgeKind) {
        let edge = GraphEdge {
            source_node_id: to_doc.to_owned(),
            target_node_id: from_doc.to_owned(),
            kind: edge_kind,
            created_at: 0,
            confidence: None,
        };
        let node = GraphNode {
            node_id: to_doc.to_owned(),
            kind: NodeKind::Document,
            project: Some(project()),
            created_at: 0,
        };
        self.neighbors
            .lock()
            .unwrap()
            .entry(from_doc.to_owned())
            .or_default()
            .push((edge, node));
    }
}

#[async_trait]
impl GraphQueryService for TestGraph {
    async fn query(&self, _: GraphQuery) -> Result<Subgraph, GraphQueryError> {
        Ok(Subgraph {
            nodes: vec![],
            edges: vec![],
        })
    }

    async fn neighbors(
        &self,
        node_id: &str,
        _: Option<EdgeKind>,
        _: TraversalDirection,
        _: usize,
    ) -> Result<Vec<(GraphEdge, GraphNode)>, GraphQueryError> {
        Ok(self
            .neighbors
            .lock()
            .unwrap()
            .get(node_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn find_edges_by_source(
        &self,
        _: &str,
        _: Option<EdgeKind>,
        _: usize,
    ) -> Result<Vec<GraphEdge>, GraphQueryError> {
        Ok(vec![])
    }

    async fn find_edges_by_target(
        &self,
        _: &str,
        _: Option<EdgeKind>,
        _: usize,
    ) -> Result<Vec<GraphEdge>, GraphQueryError> {
        Ok(vec![])
    }

    async fn shortest_path(
        &self,
        _: &str,
        _: &str,
        _: Option<EdgeKind>,
        _: u32,
    ) -> Result<Option<Subgraph>, GraphQueryError> {
        Ok(None)
    }
}

struct TermInjector {
    terms: Vec<String>,
}

#[async_trait]
impl GraphExpansionHook for TermInjector {
    async fn expand(&self, _query: &str, _results: &[RetrievalResult]) -> Vec<String> {
        self.terms.clone()
    }
}

fn make_result(doc_id: &str, text: &str, score: f64) -> RetrievalResult {
    RetrievalResult {
        chunk: ChunkRecord {
            chunk_id: ChunkId::new(format!("{doc_id}_0")),
            document_id: KnowledgeDocumentId::new(doc_id),
            source_id: SourceId::new("src_gx"),
            source_type: SourceType::PlainText,
            project: project(),
            text: text.to_owned(),
            position: 0,
            created_at: 0,
            updated_at: None,
            provenance_metadata: None,
            credibility_score: None,
            graph_linkage: None,
            embedding: None,
            content_hash: None,
            entities: vec![],
            embedding_model_id: None,
            needs_reembed: false,
        },
        score,
        breakdown: ScoringBreakdown::default(),
    }
}

async fn setup_retrieval() -> InMemoryRetrieval {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(
        store.clone(),
        ParagraphChunker {
            max_chunk_size: 200,
        },
    );

    pipeline
        .submit(ingest_req(
            "doc_a",
            "Rust ownership model prevents data races.\n\n\
         The borrow checker validates references at compile time.\n\n\
         Memory safety is guaranteed without garbage collection.",
        ))
        .await
        .unwrap();

    pipeline
        .submit(ingest_req(
            "doc_b",
            "Systems programming requires careful memory management.\n\n\
         C++ and Rust both target systems-level development.\n\n\
         Ownership and RAII patterns prevent resource leaks.",
        ))
        .await
        .unwrap();

    pipeline
        .submit(ingest_req(
            "doc_c",
            "Concurrency in systems languages is inherently unsafe.\n\n\
         Data races occur when threads share mutable state.\n\n\
         Rust fearless concurrency eliminates data races by design.",
        ))
        .await
        .unwrap();

    InMemoryRetrieval::new(store)
}

// ── (1)+(2)+(3)+(4) Ingest, build graph, expand, verify related docs ──────

#[tokio::test]
async fn graph_expansion_discovers_related_documents() {
    let graph = TestGraph::new();
    graph.add_link("doc_a", "doc_b", EdgeKind::DerivedFrom);
    graph.add_link("doc_a", "doc_c", EdgeKind::Cited);

    let hook = GraphBackedExpansion::new(graph).with_max_expansions(5);
    let results = vec![make_result("doc_a", "Rust ownership model", 0.9)];
    let expansions = hook.expand("ownership memory safety", &results).await;

    assert_eq!(expansions.len(), 2, "both related documents must be found");
    assert!(expansions.contains(&"doc_b".to_owned()));
    assert!(expansions.contains(&"doc_c".to_owned()));
}

#[tokio::test]
async fn expansion_follows_all_supported_edge_kinds() {
    let graph = TestGraph::new();
    graph.add_link("doc_x", "doc_derived", EdgeKind::DerivedFrom);
    graph.add_link("doc_x", "doc_cited", EdgeKind::Cited);
    graph.add_link("doc_x", "doc_read", EdgeKind::ReadFrom);
    graph.add_link("doc_x", "doc_embedded", EdgeKind::EmbeddedAs);

    let hook = GraphBackedExpansion::new(graph).with_max_expansions(10);
    let results = vec![make_result("doc_x", "test", 0.8)];
    let expansions = hook.expand("test", &results).await;

    assert_eq!(
        expansions.len(),
        4,
        "all four edge kinds must yield expansions"
    );
}

// ── (5) Expansion terms injected into hop 1 sub_query ────────────────────

#[tokio::test]
async fn expansion_terms_appear_in_hop_sub_queries() {
    let retrieval = setup_retrieval().await;
    let search = IterativeDeepSearch::new(retrieval)
        .with_graph_hook(TermInjector {
            terms: vec!["concurrency".to_owned(), "safety".to_owned()],
        })
        .with_quality_gate(QualityGateConfig {
            min_score_threshold: 0.99,
            min_results: 100,
        });

    let response = search
        .search(DeepSearchRequest {
            project: project(),
            query_text: "ownership".to_owned(),
            max_hops: 2,
            per_hop_limit: 5,
            mode: RetrievalMode::LexicalOnly,
        })
        .await
        .unwrap();

    assert_eq!(response.hops.len(), 2);
    let hop1_query = &response.hops[1].sub_query;
    assert!(
        hop1_query.contains("concurrency") || hop1_query.contains("safety"),
        "hop 1 must include injected terms: '{hop1_query}'"
    );
}

// ── (6) max_expansions limits output ─────────────────────────────────────

#[tokio::test]
async fn max_expansions_limits_graph_expansion_output() {
    let graph = TestGraph::new();
    for i in 0..10u32 {
        graph.add_link("doc_hub", &format!("related_{i}"), EdgeKind::DerivedFrom);
    }

    let hook = GraphBackedExpansion::new(graph).with_max_expansions(3);
    let results = vec![make_result("doc_hub", "hub doc", 0.9)];
    let expansions = hook.expand("hub", &results).await;

    assert!(
        expansions.len() <= 3,
        "max_expansions=3 must cap at 3, got {}",
        expansions.len()
    );
}

// ── (7) Deduplication ────────────────────────────────────────────────────

#[tokio::test]
async fn expansion_deduplicates_related_nodes() {
    let graph = TestGraph::new();
    for src in &["doc_a", "doc_b"] {
        graph.add_link(src, "doc_shared", EdgeKind::DerivedFrom);
    }

    let hook = GraphBackedExpansion::new(graph).with_max_expansions(10);
    let results = vec![
        make_result("doc_a", "content a", 0.9),
        make_result("doc_b", "content b", 0.8),
    ];
    let expansions = hook.expand("test", &results).await;
    let shared_count = expansions
        .iter()
        .filter(|e| e.as_str() == "doc_shared")
        .count();
    assert_eq!(shared_count, 1, "doc_shared must appear exactly once");
}

// ── (8) MockGraphExpansion drives multi-hop queries ───────────────────────

#[tokio::test]
async fn mock_expansion_injects_term_into_hop_1_sub_query() {
    let retrieval = setup_retrieval().await;
    let search = IterativeDeepSearch::new(retrieval)
        .with_graph_hook(TermInjector {
            terms: vec!["fearless".to_owned()],
        })
        .with_quality_gate(QualityGateConfig {
            min_score_threshold: 0.99,
            min_results: 100,
        });

    let response = search
        .search(DeepSearchRequest {
            project: project(),
            query_text: "rust".to_owned(),
            max_hops: 2,
            per_hop_limit: 10,
            mode: RetrievalMode::LexicalOnly,
        })
        .await
        .unwrap();

    assert_eq!(response.hops.len(), 2);
    assert!(
        response.hops[1].sub_query.contains("fearless"),
        "hop 1 must contain injected term 'fearless': '{}'",
        response.hops[1].sub_query
    );
}

// ── max_hops respected ────────────────────────────────────────────────────

#[tokio::test]
async fn max_hops_bounds_number_of_hops() {
    let retrieval = setup_retrieval().await;
    let search = IterativeDeepSearch::new(retrieval)
        .with_graph_hook(TermInjector {
            terms: vec!["extra".to_owned()],
        })
        .with_quality_gate(QualityGateConfig {
            min_score_threshold: 0.99,
            min_results: 100,
        });

    for max_hops in [1u32, 2, 3] {
        let r = setup_retrieval().await;
        let s = IterativeDeepSearch::new(r)
            .with_graph_hook(TermInjector {
                terms: vec!["extra".to_owned()],
            })
            .with_quality_gate(QualityGateConfig {
                min_score_threshold: 0.99,
                min_results: 100,
            });

        let response = s
            .search(DeepSearchRequest {
                project: project(),
                query_text: "rust".to_owned(),
                max_hops,
                per_hop_limit: 5,
                mode: RetrievalMode::LexicalOnly,
            })
            .await
            .unwrap();

        assert!(
            response.hops.len() <= max_hops as usize,
            "hops ({}) must not exceed max_hops ({})",
            response.hops.len(),
            max_hops
        );
    }
    let _ = search; // consume
}

// ── No graph neighbors → no expansion terms ───────────────────────────────

#[tokio::test]
async fn empty_graph_produces_no_expansion_terms() {
    let hook = GraphBackedExpansion::new(TestGraph::new());
    let results = vec![make_result("doc_isolated", "isolated", 0.9)];
    let expansions = hook.expand("any", &results).await;
    assert!(
        expansions.is_empty(),
        "empty graph must produce no expansion terms"
    );
}
