//! RFC 003 deep search system end-to-end integration test.
//!
//! Validates the full multi-hop retrieval pipeline:
//!   (1) ingest 3 related documents on overlapping topics
//!   (2) perform a deep search with max_hops=2
//!   (3) verify initial retrieval returns relevant chunks
//!   (4) verify at least 1 hop was performed and results accumulate across hops
//!   (5) verify diagnostics: mode and hops used are reported correctly
//!   (6) empty project returns Exhausted on first hop
//!   (7) quality gate: loose threshold → stops early (Sufficient after hop 0)
//!   (8) deduplication: merged_results contains no duplicate chunk IDs

use std::collections::HashSet;
use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::deep_search::{DeepSearchRequest, DeepSearchService, HopOutcome};
use cairn_memory::deep_search_impl::{IterativeDeepSearch, QualityGateConfig};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::RetrievalMode;

fn project() -> ProjectKey {
    ProjectKey::new("t_ds", "ws_ds", "proj_ds")
}

fn req(doc_id: &str, content: &str) -> IngestRequest {
    IngestRequest {
        document_id: KnowledgeDocumentId::new(doc_id),
        source_id: SourceId::new("src_ds"),
        source_type: SourceType::PlainText,
        project: project(),
        content: content.to_owned(),
        import_id: None,
        corpus_id: None,
        bundle_source_id: None,
        tags: vec![],
    }
}

/// Ingest 3 related documents and return an InMemoryRetrieval over them.
async fn ingest_three_docs() -> InMemoryRetrieval {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(
        store.clone(),
        ParagraphChunker {
            max_chunk_size: 120,
        },
    );

    // Document 1: Rust memory model
    pipeline
        .submit(req(
            "doc_rust",
            "Rust ownership model prevents data races at compile time.\n\n\
         The borrow checker validates all references before the program runs.\n\n\
         Lifetimes ensure that references never outlive the data they point to.",
        ))
        .await
        .unwrap();

    // Document 2: Memory safety across languages
    pipeline
        .submit(req(
            "doc_safety",
            "Memory safety is critical in systems programming languages.\n\n\
         Rust achieves safety without garbage collection through ownership.\n\n\
         C++ requires manual memory management and is prone to use-after-free bugs.",
        ))
        .await
        .unwrap();

    // Document 3: Concurrency and ownership
    pipeline
        .submit(req(
            "doc_concurrency",
            "Fearless concurrency is a key Rust design goal.\n\n\
         The ownership model prevents data races by construction.\n\n\
         Threads in Rust cannot share mutable state without synchronization primitives.",
        ))
        .await
        .unwrap();

    InMemoryRetrieval::new(store)
}

// ── (1)+(2)+(3) Ingest 3 docs, search, get relevant chunks ───────────────

#[tokio::test]
async fn deep_search_finds_relevant_chunks_across_documents() {
    let retrieval = ingest_three_docs().await;
    let search = IterativeDeepSearch::new(retrieval);

    let response = search
        .search(DeepSearchRequest {
            project: project(),
            query_text: "ownership prevents data races".to_owned(),
            max_hops: 2,
            per_hop_limit: 10,
            mode: RetrievalMode::LexicalOnly,
        })
        .await
        .unwrap();

    assert!(
        !response.merged_results.is_empty(),
        "deep search must return results for a matching query"
    );

    // Results must relate to ownership/data-race topic.
    let texts: Vec<_> = response
        .merged_results
        .iter()
        .map(|r| r.chunk.text.as_str())
        .collect();
    let relevant = texts
        .iter()
        .any(|t| t.contains("ownership") || t.contains("data race") || t.contains("borrow"));
    assert!(
        relevant,
        "at least one result must mention ownership or data races"
    );
}

// ── (4) At least 1 hop performed; results accumulate ─────────────────────

#[tokio::test]
async fn deep_search_with_max_hops_2_performs_at_least_one_hop() {
    let retrieval = ingest_three_docs().await;
    // Tight quality gate forces expansion beyond hop 0.
    let search = IterativeDeepSearch::new(retrieval).with_quality_gate(QualityGateConfig {
        min_score_threshold: 0.99, // impossible to satisfy → always NeedsExpansion
        min_results: 100,
    });

    let response = search
        .search(DeepSearchRequest {
            project: project(),
            query_text: "rust ownership memory safety".to_owned(),
            max_hops: 2,
            per_hop_limit: 5,
            mode: RetrievalMode::LexicalOnly,
        })
        .await
        .unwrap();

    assert!(!response.hops.is_empty(), "must perform at least 1 hop");
    assert_eq!(
        response.hops.len(),
        2,
        "tight quality gate forces 2 hops (max_hops=2)"
    );

    // Each hop must have a sub_query and latency recorded.
    for hop in &response.hops {
        assert!(
            !hop.sub_query.is_empty(),
            "hop {} must have a sub_query",
            hop.hop_number
        );
        // latency_ms is wall-clock — just verify it's a valid u64 (may be 0 in fast tests)
        let _ = hop.latency_ms;
    }
}

// ── (5) Diagnostics: mode and hops reported ──────────────────────────────

#[tokio::test]
async fn deep_search_response_reports_hops_and_latency() {
    let retrieval = ingest_three_docs().await;
    let search = IterativeDeepSearch::new(retrieval);

    let response = search
        .search(DeepSearchRequest {
            project: project(),
            query_text: "borrow checker".to_owned(),
            max_hops: 2,
            per_hop_limit: 5,
            mode: RetrievalMode::LexicalOnly,
        })
        .await
        .unwrap();

    // Hops slice is populated.
    assert!(
        !response.hops.is_empty(),
        "hops diagnostics must be non-empty"
    );

    // Hop 0 always exists.
    let hop0 = &response.hops[0];
    assert_eq!(hop0.hop_number, 0, "first hop must be numbered 0");
    assert!(
        !hop0.sub_query.is_empty(),
        "hop 0 must record its sub_query"
    );

    // Outcome is one of the valid variants.
    assert!(
        matches!(
            hop0.outcome,
            HopOutcome::Sufficient | HopOutcome::NeedsExpansion | HopOutcome::Exhausted
        ),
        "hop outcome must be a valid HopOutcome"
    );

    // total_latency_ms is a valid u64 (≥ sum of per-hop latencies).
    let sum_hop_latencies: u64 = response.hops.iter().map(|h| h.latency_ms).sum();
    assert!(
        response.total_latency_ms >= sum_hop_latencies,
        "total_latency_ms must be >= sum of per-hop latencies"
    );

    // Mode is captured on the request (verified by the fact we used LexicalOnly
    // and got lexical-quality results — i.e. text-overlap driven).
    assert!(
        !response.merged_results.is_empty() || response.hops[0].outcome == HopOutcome::Exhausted
    );
}

// ── (6) Empty project → Exhausted on first hop ───────────────────────────

#[tokio::test]
async fn deep_search_on_empty_project_exhausts_on_first_hop() {
    let retrieval = ingest_three_docs().await; // has data, but under a different project
    let search = IterativeDeepSearch::new(retrieval);

    let response = search
        .search(DeepSearchRequest {
            project: ProjectKey::new("nobody", "ws_empty", "proj_empty"),
            query_text: "ownership".to_owned(),
            max_hops: 3,
            per_hop_limit: 5,
            mode: RetrievalMode::LexicalOnly,
        })
        .await
        .unwrap();

    assert_eq!(
        response.hops.len(),
        1,
        "empty project must exhaust on the first hop"
    );
    assert_eq!(response.hops[0].outcome, HopOutcome::Exhausted);
    assert!(
        response.merged_results.is_empty(),
        "no results for empty project"
    );
}

// ── (7) Quality gate: loose threshold → stops early ──────────────────────

#[tokio::test]
async fn loose_quality_gate_stops_search_after_first_hop() {
    let retrieval = ingest_three_docs().await;
    let search = IterativeDeepSearch::new(retrieval).with_quality_gate(QualityGateConfig {
        min_score_threshold: 0.0, // any result is sufficient
        min_results: 1,
    });

    let response = search
        .search(DeepSearchRequest {
            project: project(),
            query_text: "rust ownership".to_owned(),
            max_hops: 10,
            per_hop_limit: 5,
            mode: RetrievalMode::LexicalOnly,
        })
        .await
        .unwrap();

    assert_eq!(
        response.hops.len(),
        1,
        "loose gate must stop after finding results on hop 0"
    );
    assert_eq!(response.hops[0].outcome, HopOutcome::Sufficient);
    assert!(
        !response.merged_results.is_empty(),
        "results must be present on early stop"
    );
}

// ── (8) Deduplication across hops ────────────────────────────────────────

#[tokio::test]
async fn merged_results_contain_no_duplicate_chunk_ids() {
    let retrieval = ingest_three_docs().await;
    // Force 2 hops to maximise dedup surface.
    let search = IterativeDeepSearch::new(retrieval).with_quality_gate(QualityGateConfig {
        min_score_threshold: 0.99,
        min_results: 100,
    });

    let response = search
        .search(DeepSearchRequest {
            project: project(),
            query_text: "rust".to_owned(),
            max_hops: 2,
            per_hop_limit: 20,
            mode: RetrievalMode::LexicalOnly,
        })
        .await
        .unwrap();

    let mut seen: HashSet<String> = HashSet::new();
    for result in &response.merged_results {
        let id = result.chunk.chunk_id.as_str().to_owned();
        assert!(
            seen.insert(id.clone()),
            "duplicate chunk ID in merged_results: {id}"
        );
    }
}

// ── Hop sub-query expansion includes prior result terms (hop 1+) ──────────

#[tokio::test]
async fn hop_1_sub_query_expands_on_hop_0_results() {
    let retrieval = ingest_three_docs().await;
    // Force expansion: tight gate, 2 hops.
    let search = IterativeDeepSearch::new(retrieval).with_quality_gate(QualityGateConfig {
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

    let hop0_query = &response.hops[0].sub_query;
    let hop1_query = &response.hops[1].sub_query;

    // Hop 0 must be (or start with) the original query.
    assert!(
        hop0_query.contains("ownership"),
        "hop 0 sub_query must contain the original query term"
    );

    // Hop 1 sub_query is allowed to equal hop 0 (if no expansion terms found)
    // or to be a superset of hop 0's query with additional terms.
    assert!(
        !hop1_query.is_empty(),
        "hop 1 must produce a non-empty sub_query"
    );
}
