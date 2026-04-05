use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::deep_search_impl::{
    GraphExpansionHook, IterativeDeepSearch, QualityGateConfig,
};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{
    RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalResult, RetrievalService,
};
use cairn_memory::deep_search::{DeepSearchRequest, DeepSearchService};

async fn ingest_docs(store: Arc<InMemoryDocumentStore>) {
    let chunker = ParagraphChunker {
        max_chunk_size: 500,
    };
    let pipeline = IngestPipeline::new(store, chunker);

    // Three similar docs about Rust memory safety, differing slightly.
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_ownership"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: ProjectKey::new("t", "w", "p"),
            content: "Rust ownership model prevents data races and ensures memory safety."
                .to_owned(),
            import_id: None,
            corpus_id: None,
            bundle_source_id: None,
            tags: vec![],
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_borrowing"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: ProjectKey::new("t", "w", "p"),
            content: "Rust borrowing rules enforce memory safety at compile time.".to_owned(),
            import_id: None,
            corpus_id: None,
            bundle_source_id: None,
            tags: vec![],
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_concurrency"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: ProjectKey::new("t", "w", "p"),
            content: "Fearless concurrency in Rust uses ownership to prevent data races."
                .to_owned(),
            import_id: None,
            corpus_id: None,
            bundle_source_id: None,
            tags: vec![],
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn mmr_reranker_promotes_diversity_in_integration() {
    let store = Arc::new(InMemoryDocumentStore::new());
    ingest_docs(store.clone()).await;
    let retrieval = InMemoryRetrieval::new(store);

    // Without MMR: results ordered purely by lexical relevance.
    let plain = retrieval
        .query(RetrievalQuery {
            project: ProjectKey::new("t", "w", "p"),
            query_text: "Rust memory safety".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    // With MMR: results reranked for diversity.
    let mmr = retrieval
        .query(RetrievalQuery {
            project: ProjectKey::new("t", "w", "p"),
            query_text: "Rust memory safety".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::Mmr,
            limit: 10,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    // Both should return results.
    assert!(plain.results.len() >= 2, "plain query should find results");
    assert!(mmr.results.len() >= 2, "MMR query should find results");

    // Diagnostics should reflect the reranker used.
    assert_eq!(mmr.diagnostics.reranker_used, RerankerStrategy::Mmr);
    assert_eq!(plain.diagnostics.reranker_used, RerankerStrategy::None);

    // MMR may reorder results vs plain relevance ordering.
    // Collect doc IDs in order for each.
    let plain_ids: Vec<&str> = plain
        .results
        .iter()
        .map(|r| r.chunk.document_id.as_str())
        .collect();
    let mmr_ids: Vec<&str> = mmr
        .results
        .iter()
        .map(|r| r.chunk.document_id.as_str())
        .collect();

    // Both should contain the same documents (just possibly reordered).
    let mut plain_sorted = plain_ids.clone();
    plain_sorted.sort();
    let mut mmr_sorted = mmr_ids.clone();
    mmr_sorted.sort();
    assert_eq!(plain_sorted, mmr_sorted, "MMR should contain same docs");
}

struct TestGraphExpansion;

#[async_trait]
impl GraphExpansionHook for TestGraphExpansion {
    async fn expand(&self, _query: &str, _results: &[RetrievalResult]) -> Vec<String> {
        // Simulate graph expansion: inject a related concept.
        vec!["concurrency".to_owned()]
    }
}

#[tokio::test]
async fn deep_search_with_graph_hook_end_to_end() {
    let store = Arc::new(InMemoryDocumentStore::new());
    ingest_docs(store.clone()).await;
    let retrieval = InMemoryRetrieval::new(store);

    let search = IterativeDeepSearch::new(retrieval)
        .with_graph_hook(TestGraphExpansion)
        .with_quality_gate(QualityGateConfig {
            min_score_threshold: 0.99,
            min_results: 100, // force expansion so hook fires
        });

    let response = search
        .search(DeepSearchRequest {
            project: ProjectKey::new("t", "w", "p"),
            query_text: "ownership".to_owned(),
            max_hops: 3,
            per_hop_limit: 10,
            mode: RetrievalMode::LexicalOnly,
        })
        .await
        .unwrap();

    // Multiple hops should have been executed.
    assert!(
        response.hops.len() > 1,
        "deep search should execute multiple hops, got {}",
        response.hops.len()
    );

    // Later hops should include graph expansion term "concurrency".
    let hop1 = &response.hops[1];
    assert!(
        hop1.sub_query.contains("concurrency"),
        "hop 1 sub_query should contain graph expansion term: {}",
        hop1.sub_query
    );

    // Results should be non-empty and deduplicated.
    assert!(
        !response.merged_results.is_empty(),
        "deep search should find results"
    );

    // Verify dedup: no duplicate chunk IDs.
    let mut seen = std::collections::HashSet::new();
    for r in &response.merged_results {
        assert!(
            seen.insert(r.chunk.chunk_id.clone()),
            "duplicate chunk_id in merged results"
        );
    }

    // Should find the concurrency doc thanks to graph expansion.
    let has_concurrency = response
        .merged_results
        .iter()
        .any(|r| r.chunk.text.to_lowercase().contains("concurrency"));
    assert!(
        has_concurrency,
        "graph expansion should help find concurrency-related content"
    );
}
