//! Integration tests for entity extraction pipeline (GAP-009).
//!
//! Tests cover: RegexEntityExtractor standalone, ingest pipeline
//! with extractor wired in, entities field on resulting ChunkRecords.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId, TenantId, WorkspaceId};
use cairn_memory::{
    entity_extraction::{
        EntityExtractionRequest, EntityExtractor, RegexEntityExtractor,
    },
    ingest::{
        ChunkRecord, IngestError, IngestRequest, IngestService, IngestStatus, SourceType,
    },
    pipeline::{DocumentStore, IngestPipeline, ParagraphChunker},
};

fn project() -> ProjectKey {
    ProjectKey::new(TenantId::new("t1"), WorkspaceId::new("w1"), "p1".to_owned())
}

// ── Minimal in-memory DocumentStore for tests ─────────────────────────────

struct MemStore {
    docs: Mutex<HashMap<String, IngestStatus>>,
    chunks: Mutex<Vec<ChunkRecord>>,
}

impl MemStore {
    fn new() -> Self {
        Self {
            docs: Mutex::new(HashMap::new()),
            chunks: Mutex::new(Vec::new()),
        }
    }

    fn get_chunks(&self) -> Vec<ChunkRecord> {
        self.chunks.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl DocumentStore for MemStore {
    async fn insert_document(
        &self,
        doc_id: &KnowledgeDocumentId,
        _source_id: &SourceId,
        _source_type: SourceType,
        _project: &ProjectKey,
        _title: Option<&str>,
    ) -> Result<(), IngestError> {
        self.docs
            .lock()
            .unwrap()
            .insert(doc_id.as_str().to_owned(), IngestStatus::Pending);
        Ok(())
    }

    async fn update_status(
        &self,
        doc_id: &KnowledgeDocumentId,
        status: IngestStatus,
    ) -> Result<(), IngestError> {
        self.docs
            .lock()
            .unwrap()
            .insert(doc_id.as_str().to_owned(), status);
        Ok(())
    }

    async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> Result<(), IngestError> {
        self.chunks.lock().unwrap().extend(chunks.iter().cloned());
        Ok(())
    }

    async fn get_status(
        &self,
        doc_id: &KnowledgeDocumentId,
    ) -> Result<Option<IngestStatus>, IngestError> {
        Ok(self.docs.lock().unwrap().get(doc_id.as_str()).copied())
    }

    async fn chunk_hashes_for_project(
        &self,
        _project: &ProjectKey,
    ) -> Result<HashSet<String>, IngestError> {
        let chunks = self.chunks.lock().unwrap();
        Ok(chunks.iter().filter_map(|c| c.content_hash.clone()).collect())
    }
}

// ── RegexEntityExtractor standalone ───────────────────────────────────────

#[test]
fn entity_extraction_persons_detected() {
    let extractor = RegexEntityExtractor::new();
    let req = EntityExtractionRequest {
        text: "Alan Turing invented the Turing machine. Grace Hopper pioneered compilers."
            .to_owned(),
        project: project(),
        extract_persons: true,
        extract_orgs: false,
        extract_locations: false,
        extract_facts: false,
    };
    let result = extractor.extract(&req);
    let persons_text = result.persons.join(" ");
    assert!(
        persons_text.contains("Alan") || persons_text.contains("Turing"),
        "expected Alan Turing in persons, got: {:?}", result.persons
    );
    assert!(
        persons_text.contains("Grace") || persons_text.contains("Hopper"),
        "expected Grace Hopper in persons, got: {:?}", result.persons
    );
}

#[test]
fn entity_extraction_orgs_with_known_suffix() {
    let extractor = RegexEntityExtractor::new();
    let req = EntityExtractionRequest {
        text: "Anthropic Inc builds AI safety systems. Google LLC operates search services."
            .to_owned(),
        project: project(),
        extract_persons: false,
        extract_orgs: true,
        extract_locations: false,
        extract_facts: false,
    };
    let result = extractor.extract(&req);
    let orgs_text = result.orgs.join(" ");
    assert!(
        orgs_text.contains("Anthropic"),
        "expected Anthropic in orgs, got: {:?}", result.orgs
    );
}

#[test]
fn entity_extraction_known_orgs_detected() {
    let extractor = RegexEntityExtractor::new();
    let req = EntityExtractionRequest {
        text: "OpenAI released GPT-4. Microsoft invested heavily in AI research."
            .to_owned(),
        project: project(),
        extract_persons: false,
        extract_orgs: true,
        extract_locations: false,
        extract_facts: false,
    };
    let result = extractor.extract(&req);
    let orgs_text = result.orgs.join(" ");
    assert!(
        orgs_text.contains("OpenAI") || orgs_text.contains("Microsoft"),
        "expected OpenAI or Microsoft in orgs, got: {:?}", result.orgs
    );
}

#[test]
fn entity_extraction_locations_via_prepositions() {
    let extractor = RegexEntityExtractor::new();
    let req = EntityExtractionRequest {
        text: "The summit was held in San Francisco with delegates from London and Tokyo."
            .to_owned(),
        project: project(),
        extract_persons: false,
        extract_orgs: false,
        extract_locations: true,
        extract_facts: false,
    };
    let result = extractor.extract(&req);
    let loc_text = result.locations.join(" ");
    assert!(
        loc_text.contains("San") || loc_text.contains("London") || loc_text.contains("Tokyo"),
        "expected at least one location, got: {:?}", result.locations
    );
}

#[test]
fn entity_extraction_facts_from_statements() {
    let extractor = RegexEntityExtractor::new();
    let req = EntityExtractionRequest {
        text: "Rust is a memory-safe programming language. \
               Python was created in 1991. \
               Java has automatic garbage collection."
            .to_owned(),
        project: project(),
        extract_persons: false,
        extract_orgs: false,
        extract_locations: false,
        extract_facts: true,
    };
    let result = extractor.extract(&req);
    assert!(
        result.facts.len() >= 2,
        "expected at least 2 facts, got: {:?}", result.facts
    );
}

#[test]
fn entity_extraction_all_entities_deduplicates() {
    let extractor = RegexEntityExtractor::new();
    let req = EntityExtractionRequest::all(
        "OpenAI builds models. OpenAI is an AI company. Alan Turing studied at Cambridge."
            .to_owned(),
        project(),
    );
    let result = extractor.extract(&req);
    let all = result.all_entities();
    // OpenAI should appear only once even if extracted multiple times
    assert_eq!(
        all.iter().filter(|e| e.to_lowercase().contains("openai")).count(),
        1,
        "OpenAI should be deduplicated in all_entities"
    );
}

#[test]
fn entity_extraction_empty_text_is_empty() {
    let extractor = RegexEntityExtractor::new();
    let req = EntityExtractionRequest::all(String::new(), project());
    let result = extractor.extract(&req);
    assert!(result.is_empty());
    assert!(!result.source_text_hash.is_empty(), "hash must always be set");
}

#[test]
fn entity_extraction_disabled_flags_return_empty_lists() {
    let extractor = RegexEntityExtractor::new();
    let req = EntityExtractionRequest {
        text: "Alan Turing worked at Anthropic Inc in London is a scientist.".to_owned(),
        project: project(),
        extract_persons: false,
        extract_orgs: false,
        extract_locations: false,
        extract_facts: false,
    };
    let result = extractor.extract(&req);
    assert!(result.persons.is_empty());
    assert!(result.orgs.is_empty());
    assert!(result.locations.is_empty());
    assert!(result.facts.is_empty());
}

// ── Ingest pipeline with entity extraction wired ─────────────────────────

#[tokio::test]
async fn entity_extraction_ingest_pipeline_populates_entities_field() {
    let store = Arc::new(MemStore::new());
    let chunker = ParagraphChunker { max_chunk_size: 500 };
    let extractor: Arc<dyn EntityExtractor> = Arc::new(RegexEntityExtractor::new());

    let pipeline = IngestPipeline::new(store.clone(), chunker)
        .with_extractor(extractor);

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_entities_1"),
            source_id: SourceId::new("src_1"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Alan Turing worked at Anthropic Inc. \
                     Marie Curie discovered polonium in Paris. \
                     OpenAI is an AI research company."
                .to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let chunks = store.get_chunks();
    assert!(!chunks.is_empty(), "pipeline must produce chunks");

    // At least one chunk must have entities populated.
    let any_entities = chunks.iter().any(|c| !c.entities.is_empty());
    assert!(
        any_entities,
        "at least one chunk must have entities after ingest with extractor, \
         got chunks: {:?}",
        chunks.iter().map(|c| (&c.text[..c.text.len().min(50)], &c.entities)).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn entity_extraction_without_extractor_leaves_entities_empty() {
    let store = Arc::new(MemStore::new());
    let chunker = ParagraphChunker { max_chunk_size: 500 };

    // No extractor wired — entities must remain empty
    let pipeline = IngestPipeline::new(store.clone(), chunker);

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_no_extractor"),
            source_id: SourceId::new("src_1"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Alan Turing worked at Anthropic Inc in London.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let chunks = store.get_chunks();
    assert!(!chunks.is_empty());
    for chunk in &chunks {
        assert!(
            chunk.entities.is_empty(),
            "entities must be empty when no extractor is configured, got: {:?}",
            chunk.entities
        );
    }
}

#[tokio::test]
async fn entity_extraction_ingest_known_names_in_entities() {
    let store = Arc::new(MemStore::new());
    let chunker = ParagraphChunker { max_chunk_size: 1000 };
    let extractor: Arc<dyn EntityExtractor> = Arc::new(RegexEntityExtractor::new());

    let pipeline = IngestPipeline::new(store.clone(), chunker)
        .with_extractor(extractor);

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_entities_2"),
            source_id: SourceId::new("src_2"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Google LLC and Microsoft Corporation are technology companies. \
                     The conference was held in San Francisco."
                .to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let chunks = store.get_chunks();
    let all_entities: Vec<String> = chunks.iter().flat_map(|c| c.entities.clone()).collect();
    let entities_text = all_entities.join(" ");

    assert!(
        entities_text.contains("Google") || entities_text.contains("Microsoft")
            || entities_text.contains("San"),
        "expected known org or location in entities, got: {:?}", all_entities
    );
}
