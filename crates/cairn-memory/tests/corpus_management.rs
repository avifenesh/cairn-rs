//! RFC 003 corpus management integration tests.

use std::sync::Arc;

use cairn_api::memory_api::{AddDocumentToCorpusRequest, CorpusEndpoints, CreateCorpusRequest};
use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::api_impl::CorpusApiImpl;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{
    MetadataFilter, RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService,
};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// Create a corpus, ingest 2 docs into it and 1 doc outside.
/// Filter by corpus_id: only 2 returned. No filter: all 3 returned.
#[tokio::test]
async fn corpus_management_filter_by_corpus_id() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());
    let api = CorpusApiImpl::new(store.clone());

    let corpus = api
        .create_corpus(
            &project(),
            &CreateCorpusRequest {
                name: "production_docs".to_owned(),
                description: Some("Docs for production".to_owned()),
            },
        )
        .await
        .unwrap();
    let corpus_id = corpus.corpus_id.clone();

    for (doc_id, src) in [("doc_corp_a", "src_a"), ("doc_corp_b", "src_b")] {
        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new(doc_id),
                source_id: SourceId::new(src),
                source_type: SourceType::PlainText,
                project: project(),
                content: format!("Rust memory safety ownership borrow checker {doc_id} corpus content"),
                tags: vec![],
                corpus_id: Some(corpus_id.clone()),
                bundle_source_id: None,
                import_id: None,
            })
            .await
            .unwrap();
    }

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_no_corpus"),
            source_id: SourceId::new("src_c"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust memory safety ownership borrow checker standalone gamma".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let filtered = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust memory safety".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 20,
            metadata_filters: vec![MetadataFilter {
                key: "corpus_id".to_owned(),
                value: corpus_id.clone(),
            }],
            scoring_policy: None,
        })
        .await
        .unwrap();

    let filtered_ids: Vec<String> = filtered.results.iter()
        .map(|r| r.chunk.document_id.to_string())
        .collect();

    assert_eq!(filtered_ids.len(), 2,
        "corpus filter should return 2 results, got {}: {:?}", filtered_ids.len(), filtered_ids);
    assert!(filtered_ids.contains(&"doc_corp_a".to_owned()));
    assert!(filtered_ids.contains(&"doc_corp_b".to_owned()));
    assert!(!filtered_ids.contains(&"doc_no_corpus".to_owned()));

    let all = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust memory safety".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 20,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert_eq!(all.results.len(), 3,
        "unfiltered search must return all 3 docs, got {}", all.results.len());
}

/// document_count increments as documents are added to the corpus.
#[tokio::test]
async fn corpus_management_document_count_tracks_membership() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let api = CorpusApiImpl::new(store.clone());

    let corpus = api
        .create_corpus(&project(), &CreateCorpusRequest { name: "count_test".to_owned(), description: None })
        .await.unwrap();

    let fetched = api.get_corpus(&corpus.corpus_id).await.unwrap().unwrap();
    assert_eq!(fetched.document_count, 0);

    for (n, doc_id) in ["doc_ct_1", "doc_ct_2", "doc_ct_3"].iter().enumerate() {
        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new(*doc_id),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: project(),
                content: format!("Unique content for document {doc_id} systems programming"),
                tags: vec![],
                corpus_id: Some(corpus.corpus_id.clone()),
                bundle_source_id: None,
                import_id: None,
            })
            .await.unwrap();

        let updated = api.get_corpus(&corpus.corpus_id).await.unwrap().unwrap();
        assert_eq!(updated.document_count, (n + 1) as u32);
    }
}

/// POST /v1/memory/corpora/:id/documents adds a document to a corpus post-ingest.
#[tokio::test]
async fn corpus_management_api_add_document_post_ingest() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());
    let api = CorpusApiImpl::new(store.clone());

    let corpus = api
        .create_corpus(&project(), &CreateCorpusRequest { name: "post_add".to_owned(), description: None })
        .await.unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_post_add"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Post-ingest corpus add unique test document content here".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await.unwrap();

    let q = RetrievalQuery {
        project: project(),
        query_text: "corpus add unique".to_owned(),
        mode: RetrievalMode::LexicalOnly,
        reranker: RerankerStrategy::None,
        limit: 10,
        metadata_filters: vec![MetadataFilter { key: "corpus_id".to_owned(), value: corpus.corpus_id.clone() }],
        scoring_policy: None,
    };

    let before = retrieval.query(q.clone()).await.unwrap();
    assert!(before.results.is_empty(), "doc not in corpus yet");

    api.add_document_to_corpus(&corpus.corpus_id, &AddDocumentToCorpusRequest {
        document_id: "doc_post_add".to_owned(),
    }).await.unwrap();

    let after = retrieval.query(q).await.unwrap();
    assert_eq!(after.results.len(), 1, "doc should appear after being added");
}

/// list_corpora returns only corpora for the given project.
#[tokio::test]
async fn corpus_management_list_corpora_scoped_to_project() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let api = CorpusApiImpl::new(store.clone());

    let proj_a = ProjectKey::new("t", "w", "proj_a");
    let proj_b = ProjectKey::new("t", "w", "proj_b");

    api.create_corpus(&proj_a, &CreateCorpusRequest { name: "corp_a1".to_owned(), description: None }).await.unwrap();
    api.create_corpus(&proj_a, &CreateCorpusRequest { name: "corp_a2".to_owned(), description: None }).await.unwrap();
    api.create_corpus(&proj_b, &CreateCorpusRequest { name: "corp_b1".to_owned(), description: None }).await.unwrap();

    let a_corpora = api.list_corpora(&proj_a).await.unwrap();
    let b_corpora = api.list_corpora(&proj_b).await.unwrap();

    assert_eq!(a_corpora.len(), 2);
    assert_eq!(b_corpora.len(), 1);
    assert_eq!(b_corpora[0].name, "corp_b1");
}
