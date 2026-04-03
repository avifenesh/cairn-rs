use async_trait::async_trait;
use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};

use crate::ingest::{
    ChunkRecord, IngestError, IngestPackRequest, IngestRequest, IngestService, IngestStatus,
    SourceType,
};

/// Provider-abstracted embedding service boundary.
///
/// Concrete implementations call hosted embedding APIs (e.g. OpenAI,
/// Bedrock, Cohere) or local embedding models. The pipeline calls
/// this after chunking to generate embeddings for each chunk.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate an embedding vector for a text chunk.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, IngestError>;
}

/// Text chunker that splits document content into retrieval-sized pieces.
///
/// The default implementation uses simple paragraph-based splitting.
/// More sophisticated chunkers (semantic, sliding-window) can be
/// plugged in later.
pub trait Chunker: Send + Sync {
    fn chunk(
        &self,
        content: &str,
        document_id: &KnowledgeDocumentId,
        source_id: &SourceId,
        source_type: SourceType,
        project: &ProjectKey,
    ) -> Vec<ChunkRecord>;
}

/// Simple paragraph-based chunker.
pub struct ParagraphChunker {
    pub max_chunk_size: usize,
}

impl Default for ParagraphChunker {
    fn default() -> Self {
        Self {
            max_chunk_size: 1000,
        }
    }
}

impl Chunker for ParagraphChunker {
    fn chunk(
        &self,
        content: &str,
        document_id: &KnowledgeDocumentId,
        source_id: &SourceId,
        source_type: SourceType,
        project: &ProjectKey,
    ) -> Vec<ChunkRecord> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut chunks = Vec::new();
        let mut current = String::new();
        let mut position: u32 = 0;

        for line in content.lines() {
            if line.trim().is_empty() && !current.is_empty() {
                if current.len() >= self.max_chunk_size {
                    chunks.push(make_chunk(
                        &current,
                        position,
                        document_id,
                        source_id,
                        source_type,
                        project,
                        now,
                    ));
                    position += 1;
                    current.clear();
                }
                current.push('\n');
                continue;
            }

            if current.len() + line.len() > self.max_chunk_size && !current.is_empty() {
                chunks.push(make_chunk(
                    &current,
                    position,
                    document_id,
                    source_id,
                    source_type,
                    project,
                    now,
                ));
                position += 1;
                current.clear();
            }

            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }

        if !current.trim().is_empty() {
            chunks.push(make_chunk(
                &current,
                position,
                document_id,
                source_id,
                source_type,
                project,
                now,
            ));
        }

        chunks
    }
}

fn make_chunk(
    text: &str,
    position: u32,
    document_id: &KnowledgeDocumentId,
    source_id: &SourceId,
    source_type: SourceType,
    project: &ProjectKey,
    created_at: u64,
) -> ChunkRecord {
    let chunk_id = ChunkId::new(format!("{}_{}", document_id.as_str(), position));
    ChunkRecord {
        chunk_id,
        document_id: document_id.clone(),
        source_id: source_id.clone(),
        source_type,
        project: project.clone(),
        text: text.trim().to_owned(),
        position,
        created_at,
        updated_at: None,
        provenance_metadata: None,
        credibility_score: None,
        graph_linkage: None,
        embedding: None,
    }
}

/// Document store trait for pipeline persistence.
///
/// Abstracts over PgDocumentStore and in-memory implementations.
#[async_trait]
pub trait DocumentStore: Send + Sync {
    async fn insert_document(
        &self,
        document_id: &KnowledgeDocumentId,
        source_id: &SourceId,
        source_type: SourceType,
        project: &ProjectKey,
        title: Option<&str>,
    ) -> Result<(), IngestError>;

    async fn update_status(
        &self,
        document_id: &KnowledgeDocumentId,
        status: IngestStatus,
    ) -> Result<(), IngestError>;

    async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> Result<(), IngestError>;

    async fn get_status(
        &self,
        document_id: &KnowledgeDocumentId,
    ) -> Result<Option<IngestStatus>, IngestError>;
}

#[async_trait]
impl<T: DocumentStore> DocumentStore for std::sync::Arc<T> {
    async fn insert_document(
        &self,
        document_id: &KnowledgeDocumentId,
        source_id: &SourceId,
        source_type: SourceType,
        project: &ProjectKey,
        title: Option<&str>,
    ) -> Result<(), IngestError> {
        (**self)
            .insert_document(document_id, source_id, source_type, project, title)
            .await
    }

    async fn update_status(
        &self,
        document_id: &KnowledgeDocumentId,
        status: IngestStatus,
    ) -> Result<(), IngestError> {
        (**self).update_status(document_id, status).await
    }

    async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> Result<(), IngestError> {
        (**self).insert_chunks(chunks).await
    }

    async fn get_status(
        &self,
        document_id: &KnowledgeDocumentId,
    ) -> Result<Option<IngestStatus>, IngestError> {
        (**self).get_status(document_id).await
    }
}

/// Concrete ingest pipeline that coordinates parse -> chunk -> embed -> persist.
pub struct IngestPipeline<S: DocumentStore, C: Chunker> {
    store: S,
    chunker: C,
}

impl<S: DocumentStore, C: Chunker> IngestPipeline<S, C> {
    pub fn new(store: S, chunker: C) -> Self {
        Self { store, chunker }
    }
}

#[async_trait]
impl<S: DocumentStore + 'static, C: Chunker + 'static> IngestService for IngestPipeline<S, C> {
    async fn submit(&self, request: IngestRequest) -> Result<(), IngestError> {
        // 1. Register document.
        self.store
            .insert_document(
                &request.document_id,
                &request.source_id,
                request.source_type,
                &request.project,
                None,
            )
            .await?;

        // 2. Mark as parsing.
        self.store
            .update_status(&request.document_id, IngestStatus::Parsing)
            .await?;

        // 3. Chunk the content.
        self.store
            .update_status(&request.document_id, IngestStatus::Chunking)
            .await?;

        let chunks = self.chunker.chunk(
            &request.content,
            &request.document_id,
            &request.source_id,
            request.source_type,
            &request.project,
        );

        // 4. Persist chunks.
        self.store
            .update_status(&request.document_id, IngestStatus::Indexing)
            .await?;
        self.store.insert_chunks(&chunks).await?;

        // 5. Mark completed.
        // (Embedding step is skipped in this skeleton — will be added
        //  when EmbeddingProvider is wired in.)
        self.store
            .update_status(&request.document_id, IngestStatus::Completed)
            .await?;

        Ok(())
    }

    async fn submit_pack(&self, request: IngestPackRequest) -> Result<(), IngestError> {
        use crate::bundles::{ArtifactKind, BundleEnvelope, BundleType};

        let bundle: BundleEnvelope = serde_json::from_str(&request.bundle_json)
            .map_err(|e| IngestError::ParseFailed(format!("invalid bundle JSON: {e}")))?;

        if bundle.bundle_type != BundleType::CuratedKnowledgePackBundle {
            return Err(IngestError::ParseFailed(format!(
                "expected curated_knowledge_pack_bundle, got {:?}",
                bundle.bundle_type
            )));
        }

        for artifact in &bundle.artifacts {
            if artifact.artifact_kind != ArtifactKind::KnowledgeDocument {
                continue;
            }

            let content = artifact.payload["content"]["text"].as_str().unwrap_or("");
            if content.is_empty() {
                continue;
            }

            let source_type = match artifact.payload["source_type"].as_str().unwrap_or("") {
                "text_plain" => SourceType::PlainText,
                "text_markdown" => SourceType::Markdown,
                "text_html" => SourceType::Html,
                "json_structured" => SourceType::StructuredJson,
                _ => SourceType::PlainText,
            };

            self.submit(IngestRequest {
                document_id: cairn_domain::KnowledgeDocumentId::new(&artifact.artifact_logical_id),
                source_id: cairn_domain::SourceId::new(&bundle.bundle_id),
                source_type,
                project: request.project.clone(),
                content: content.to_owned(),
            })
            .await?;
        }

        Ok(())
    }

    async fn status(
        &self,
        document_id: &KnowledgeDocumentId,
    ) -> Result<Option<IngestStatus>, IngestError> {
        self.store.get_status(document_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MemDocStore {
        docs: Mutex<HashMap<String, IngestStatus>>,
        chunks: Mutex<Vec<ChunkRecord>>,
    }

    impl MemDocStore {
        fn new() -> Self {
            Self {
                docs: Mutex::new(HashMap::new()),
                chunks: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl DocumentStore for MemDocStore {
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
    }

    #[tokio::test]
    async fn ingest_pipeline_chunks_and_completes() {
        let store = MemDocStore::new();
        let chunker = ParagraphChunker { max_chunk_size: 50 };
        let pipeline = IngestPipeline::new(store, chunker);

        let request = IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_1"),
            source_id: SourceId::new("src_1"),
            source_type: SourceType::PlainText,
            project: ProjectKey::new("t", "w", "p"),
            content: "Hello world.\n\nThis is a test document.\n\nIt has multiple paragraphs."
                .to_owned(),
        };

        pipeline.submit(request).await.unwrap();

        let status = pipeline
            .status(&KnowledgeDocumentId::new("doc_1"))
            .await
            .unwrap();
        assert_eq!(status, Some(IngestStatus::Completed));

        let chunks = pipeline.store.chunks.lock().unwrap();
        assert!(chunks.len() >= 1);
        assert!(chunks[0].text.contains("Hello"));
    }

    #[test]
    fn paragraph_chunker_respects_max_size() {
        let chunker = ParagraphChunker { max_chunk_size: 30 };
        let doc_id = KnowledgeDocumentId::new("d1");
        let src_id = SourceId::new("s1");
        let project = ProjectKey::new("t", "w", "p");

        let chunks = chunker.chunk(
            "Short line.\n\nAnother short line.\n\nThird line here.",
            &doc_id,
            &src_id,
            SourceType::PlainText,
            &project,
        );

        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.text.len() <= 40); // some slack for line boundaries
        }
    }
}
