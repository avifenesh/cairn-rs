use async_trait::async_trait;
use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::entity_extraction::{EntityExtractionRequest, EntityExtractor};
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

/// No-op embedding provider that returns empty vectors.
///
/// Used as the default when no embedding provider is configured.
/// Chunks will have `embedding: None` when this provider is active.
pub struct NoOpEmbeddingProvider;

#[async_trait]
impl EmbeddingProvider for NoOpEmbeddingProvider {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>, IngestError> {
        Ok(vec![])
    }
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
                    // Buffer was just cleared: do NOT append '\n' or we'd create
                    // a whitespace-only ghost chunk on the next line.
                } else {
                    // Paragraph too short to emit yet; preserve the blank-line
                    // separator so adjacent paragraphs stay readable if merged.
                    current.push('\n');
                }
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

/// Convert a `BundleSourceType` (or `None`) to an ingest `SourceType`.
///
/// Used by import service to map bundle artifact source types to the
/// ingest pipeline source types.
pub fn bundle_source_type_to_ingest(
    bundle_source_type: Option<crate::bundles::BundleSourceType>,
) -> crate::ingest::SourceType {
    use crate::bundles::BundleSourceType;
    use crate::ingest::SourceType;
    match bundle_source_type {
        Some(BundleSourceType::TextMarkdown) => SourceType::Markdown,
        Some(BundleSourceType::TextHtml) => SourceType::Html,
        Some(BundleSourceType::JsonStructured) => SourceType::StructuredJson,
        Some(BundleSourceType::ExternalRef) => SourceType::PlainText,
        Some(BundleSourceType::TextPlain) | None => SourceType::PlainText,
    }
}

/// Extract the text content from a bundle artifact's JSON payload.
///
/// Handles `DocumentContent::InlineText`, `InlineJson`, and falls back to
/// a raw string conversion. Returns `None` if no text content can be extracted.
pub fn extract_document_content_text(payload: &serde_json::Value) -> Option<String> {
    // Try `content.text` for InlineText variant.
    if let Some(text) = payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
    {
        return Some(text.to_owned());
    }
    // Try top-level `text` field.
    if let Some(text) = payload.get("text").and_then(|t| t.as_str()) {
        return Some(text.to_owned());
    }
    // Try `content.value` for InlineJson variant.
    if let Some(value) = payload.get("content").and_then(|c| c.get("value")) {
        return Some(value.to_string());
    }
    None
}

/// Compute a stable content hash for deduplication.
pub fn compute_content_hash(text: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Compute a chunk quality score from text characteristics.
///
/// Score = (alphanumeric_ratio * 0.7 + length_factor * 0.3) * provenance_boost.
/// Provenance-bearing chunks get a 1.2x boost (capped at 1.0);
/// chunks without provenance are penalized to 0.8x.
fn compute_chunk_quality(text: &str, has_provenance: bool) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let alnum = text.chars().filter(|c| c.is_alphanumeric()).count() as f64 / text.len() as f64;
    let length_factor = (text.len() as f64 / 100.0).min(1.0);
    let base = alnum * 0.7 + length_factor * 0.3;
    if has_provenance {
        (base * 1.2).min(1.0)
    } else {
        base * 0.8
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
    let trimmed = text.trim().to_owned();
    let content_hash = compute_content_hash(&trimmed);

    let provenance = serde_json::json!({
        "document_id": document_id.as_str(),
        "source_id": source_id.as_str(),
        "source_type": format!("{source_type:?}"),
        "position": position,
        "created_at": created_at,
    });

    let quality = compute_chunk_quality(&trimmed, true);

    ChunkRecord {
        chunk_id,
        document_id: document_id.clone(),
        source_id: source_id.clone(),
        source_type,
        project: project.clone(),
        text: trimmed,
        position,
        created_at,
        updated_at: None,
        provenance_metadata: Some(provenance),
        credibility_score: Some(quality),
        graph_linkage: None,
        embedding: None,
        content_hash: Some(content_hash),
        entities: Vec::new(),
        embedding_model_id: None,
        needs_reembed: false,
    }
}

/// Normalize raw content based on source type into portable plain text.
///
/// Per RFC 003, v1 must normalize supported source types into portable
/// owned retrieval documents rather than keeping parser-specific blobs.
pub fn normalize(content: &str, source_type: SourceType) -> String {
    match source_type {
        SourceType::PlainText | SourceType::KnowledgePack | SourceType::JsonStructured => {
            content.to_owned()
        }
        SourceType::Html => strip_html(content),
        SourceType::Markdown => strip_markdown(content),
        SourceType::StructuredJson => extract_json_text(content),
    }
}

/// Strip HTML tags, decode common entities, collapse whitespace.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '<' => {
                // Consume everything until '>' (tag content is discarded).
                // Insert newline for block-level elements.
                let mut tag = String::new();
                for tc in chars.by_ref() {
                    if tc == '>' {
                        break;
                    }
                    tag.push(tc);
                }
                let tag_lower = tag.trim().to_lowercase();
                let tag_name = tag_lower
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next()
                    .unwrap_or("");
                if matches!(
                    tag_name,
                    "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                ) {
                    out.push('\n');
                }
            }
            '&' => {
                let mut entity = String::new();
                for ec in chars.by_ref() {
                    if ec == ';' {
                        break;
                    }
                    entity.push(ec);
                    if entity.len() > 8 {
                        break;
                    }
                }
                match entity.as_str() {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    "nbsp" => out.push(' '),
                    _ => {
                        out.push('&');
                        out.push_str(&entity);
                    }
                }
            }
            _ => out.push(ch),
        }
    }

    // Collapse multiple blank lines.
    collapse_whitespace(&out)
}

/// Strip markdown formatting: headers, links, emphasis, code fences.
fn strip_markdown(md: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut in_code_fence = false;

    for line in md.lines() {
        let trimmed = line.trim();

        // Toggle code fences.
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            continue;
        }

        if in_code_fence {
            lines.push(line.to_owned());
            continue;
        }

        // Strip header markers.
        let stripped = if trimmed.starts_with('#') {
            trimmed.trim_start_matches('#').trim().to_owned()
        } else {
            line.to_owned()
        };

        // Strip inline formatting.
        let stripped = strip_md_inline(&stripped);
        lines.push(stripped);
    }

    collapse_whitespace(&lines.join("\n"))
}

/// Strip inline markdown: links, images, bold, italic, inline code.
fn strip_md_inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            // Links: [text](url) → text
            // Images: ![alt](url) → alt
            '!' if chars.peek() == Some(&'[') => {
                chars.next(); // consume '['
                let mut text = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    text.push(c);
                }
                // Skip (url) part.
                if chars.peek() == Some(&'(') {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
                out.push_str(&text);
            }
            '[' => {
                let mut text = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    text.push(c);
                }
                // Skip (url) part.
                if chars.peek() == Some(&'(') {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
                out.push_str(&text);
            }
            // Bold/italic markers: skip * and _
            '*' | '_' => {
                // Skip consecutive markers.
                while chars.peek() == Some(&ch) {
                    chars.next();
                }
            }
            // Inline code: `code` → code
            '`' => {
                while chars.peek() == Some(&'`') {
                    chars.next();
                }
            }
            _ => out.push(ch),
        }
    }

    out
}

/// Extract text values from a JSON document.
fn extract_json_text(json_str: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return json_str.to_owned();
    };

    let mut texts = Vec::new();
    collect_json_strings(&value, &mut texts);

    if texts.is_empty() {
        // Fallback: pretty-print.
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| json_str.to_owned())
    } else {
        texts.join("\n")
    }
}

fn collect_json_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) if !s.trim().is_empty() => out.push(s.clone()),
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_json_strings(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values() {
                collect_json_strings(v, out);
            }
        }
        _ => {}
    }
}

/// Collapse runs of blank lines into single blank lines and trim.
fn collapse_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_blank = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank && !result.is_empty() {
                result.push('\n');
            }
            prev_blank = true;
        } else {
            if prev_blank && !result.is_empty() {
                result.push('\n');
            }
            result.push_str(trimmed);
            result.push('\n');
            prev_blank = false;
        }
    }

    result.trim().to_owned()
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

    /// Return content hashes of all existing chunks in a project for dedup.
    async fn chunk_hashes_for_project(
        &self,
        project: &ProjectKey,
    ) -> Result<HashSet<String>, IngestError>;
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

    async fn chunk_hashes_for_project(
        &self,
        project: &ProjectKey,
    ) -> Result<HashSet<String>, IngestError> {
        (**self).chunk_hashes_for_project(project).await
    }
}

/// Concrete ingest pipeline that coordinates parse -> chunk -> embed -> extract -> persist.
pub struct IngestPipeline<S: DocumentStore, C: Chunker> {
    store: S,
    chunker: C,
    embedder: Arc<dyn EmbeddingProvider>,
    extractor: Option<Arc<dyn EntityExtractor>>,
    /// Logical ID of the embedding model in use.
    ///
    /// Stored on each `ChunkRecord::embedding_model_id` so callers can
    /// detect when the model has changed and trigger `re_embed_all`.
    embedding_model_id: Option<String>,
}

impl<S: DocumentStore, C: Chunker> IngestPipeline<S, C> {
    pub fn new(store: S, chunker: C) -> Self {
        Self {
            store,
            chunker,
            embedder: Arc::new(NoOpEmbeddingProvider),
            extractor: None,
            embedding_model_id: None,
        }
    }

    /// Set a concrete embedding provider. When set, the pipeline generates
    /// embeddings for each chunk during ingest.
    pub fn with_embedder(mut self, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        self.embedder = embedder;
        self
    }

    /// Set the logical embedding model ID recorded on each embedded chunk.
    ///
    /// Used by `re_embed_all` to detect stale embeddings when the model changes.
    pub fn with_embedding_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.embedding_model_id = Some(model_id.into());
        self
    }

    /// Set an entity extractor. When set, the pipeline runs entity extraction
    /// on each chunk and populates `ChunkRecord::entities`.
    pub fn with_extractor(mut self, extractor: Arc<dyn EntityExtractor>) -> Self {
        self.extractor = Some(extractor);
        self
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

        // 2. Parse and normalize content based on source type.
        self.store
            .update_status(&request.document_id, IngestStatus::Parsing)
            .await?;

        let normalized = normalize(&request.content, request.source_type);

        // 3. Chunk the normalized content.
        self.store
            .update_status(&request.document_id, IngestStatus::Chunking)
            .await?;

        let mut chunks = self.chunker.chunk(
            &normalized,
            &request.document_id,
            &request.source_id,
            request.source_type,
            &request.project,
        );

        // 3b. Propagate corpus_id, tags, external_ref, and retrieval hints from IngestRequest
        //     into chunk provenance_metadata.
        //
        //     Tags prefixed with "__hint:" are decoded into provenance_metadata.hints rather
        //     than stored as plain tags; this lets submit_pack pass hints without adding a new
        //     field to IngestRequest.
        let plain_tags: Vec<&str> = request
            .tags
            .iter()
            .filter(|t| !t.starts_with("__hint:"))
            .map(String::as_str)
            .collect();
        let hints: Vec<&str> = request
            .tags
            .iter()
            .filter_map(|t| t.strip_prefix("__hint:"))
            .collect();

        let needs_meta = request.corpus_id.is_some()
            || request.bundle_source_id.is_some()
            || !plain_tags.is_empty()
            || !hints.is_empty();

        if needs_meta {
            for chunk in &mut chunks {
                let mut meta = chunk
                    .provenance_metadata
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({}));
                if let Some(obj) = meta.as_object_mut() {
                    if let Some(ref cid) = request.corpus_id {
                        obj.insert("corpus_id".to_owned(), serde_json::json!(cid));
                    }
                    // bundle_source_id carries the external_ref URI from submit_pack.
                    if let Some(ref ext_ref) = request.bundle_source_id {
                        obj.insert("external_ref".to_owned(), serde_json::json!(ext_ref));
                    }
                    if !plain_tags.is_empty() {
                        obj.insert("tags".to_owned(), serde_json::json!(plain_tags));
                    }
                    if !hints.is_empty() {
                        obj.insert("hints".to_owned(), serde_json::json!(hints));
                    }
                }
                chunk.provenance_metadata = Some(meta);
            }
        }

        // 4. Dedup: remove chunks whose content hash already exists in this project
        //    (cross-batch) or that appeared earlier in this same batch (within-batch).
        let existing_hashes = self
            .store
            .chunk_hashes_for_project(&request.project)
            .await?;
        let mut batch_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        chunks.retain(|c| match c.content_hash.as_ref() {
            None => true,
            Some(h) => {
                if existing_hashes.contains(h) || batch_seen.contains(h) {
                    false
                } else {
                    batch_seen.insert(h.clone());
                    true
                }
            }
        });

        // 5. Generate embeddings for each chunk.
        self.store
            .update_status(&request.document_id, IngestStatus::Embedding)
            .await?;

        for chunk in &mut chunks {
            let embedding = self.embedder.embed(&chunk.text).await?;
            if !embedding.is_empty() {
                chunk.embedding = Some(embedding);
                chunk.embedding_model_id = self.embedding_model_id.clone();
                chunk.needs_reembed = false;
            }
        }

        // 5b. Run entity extraction on each chunk when an extractor is configured.
        if let Some(extractor) = &self.extractor {
            for chunk in &mut chunks {
                let req = EntityExtractionRequest::all(chunk.text.clone(), request.project.clone());
                let result = extractor.extract(&req);
                chunk.entities = result.all_entities();
            }
        }

        // 6. Persist chunks.
        self.store
            .update_status(&request.document_id, IngestStatus::Indexing)
            .await?;
        self.store.insert_chunks(&chunks).await?;

        // 7. Mark completed.
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

            // Try to deserialize the artifact payload as the typed struct so we can access
            // all content variants (InlineText, InlineJson, ExternalRef) and retrieval hints.
            let typed: Option<crate::bundles::KnowledgeDocumentPayload> =
                serde_json::from_value(artifact.payload.as_value()).ok();

            // Determine content text and external_ref URI from the typed payload.
            // Falls back to raw JSON field access for backward compatibility.
            let (content, external_ref_uri) = if let Some(ref kd) = typed {
                match &kd.content {
                    crate::bundles::DocumentContent::InlineText { text } => (text.clone(), None),
                    crate::bundles::DocumentContent::InlineJson { value } => {
                        (value.to_string(), None)
                    }
                    crate::bundles::DocumentContent::ExternalRef { uri, .. } => {
                        // Use the URI as a minimal text placeholder so the chunk
                        // records where the content lives; full content is in provenance.
                        (uri.clone(), Some(uri.clone()))
                    }
                }
            } else {
                // Legacy raw-JSON fallback.
                let pv = artifact.payload.as_value();
                let text = pv["content"]["text"].as_str().unwrap_or("").to_owned();
                (text, None)
            };

            if content.is_empty() {
                continue;
            }

            // Retrieval hints from the typed payload, encoded as prefixed tags so that
            // step 3b can write them to provenance_metadata.hints without requiring a
            // new field on IngestRequest.
            let mut tags: Vec<String> = vec![];
            if let Some(ref kd) = typed {
                for hint in &kd.retrieval_hints {
                    tags.push(format!("__hint:{hint}"));
                }
            }

            let source_type = if let Some(ref kd) = typed {
                match kd.source_type {
                    crate::bundles::BundleSourceType::TextMarkdown => SourceType::Markdown,
                    crate::bundles::BundleSourceType::TextHtml => SourceType::Html,
                    crate::bundles::BundleSourceType::JsonStructured => SourceType::StructuredJson,
                    _ => SourceType::PlainText,
                }
            } else {
                let pv = artifact.payload.as_value();
                match pv["source_type"].as_str().unwrap_or("") {
                    "text_plain" => SourceType::PlainText,
                    "text_markdown" => SourceType::Markdown,
                    "text_html" => SourceType::Html,
                    "json_structured" => SourceType::StructuredJson,
                    _ => SourceType::PlainText,
                }
            };

            self.submit(IngestRequest {
                document_id: cairn_domain::KnowledgeDocumentId::new(&artifact.artifact_logical_id),
                source_id: cairn_domain::SourceId::new(&bundle.bundle_id),
                source_type,
                project: request.project.clone(),
                content,
                import_id: None,
                corpus_id: None,
                // bundle_source_id carries the external_ref URI when present so that
                // step 3b can write it to provenance_metadata.external_ref.
                bundle_source_id: external_ref_uri,
                tags,
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

        async fn chunk_hashes_for_project(
            &self,
            _project: &ProjectKey,
        ) -> Result<HashSet<String>, IngestError> {
            let chunks = self.chunks.lock().unwrap();
            let hashes = chunks
                .iter()
                .filter_map(|c| c.content_hash.clone())
                .collect();
            Ok(hashes)
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
            import_id: None,
            corpus_id: None,
            bundle_source_id: None,
            tags: vec![],
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

    #[test]
    fn normalize_plain_text_passes_through() {
        let input = "Hello world.\n\nSecond paragraph.";
        let result = super::normalize(input, SourceType::PlainText);
        assert_eq!(result, input);
    }

    #[test]
    fn normalize_html_strips_tags_and_decodes_entities() {
        let html = "<h1>Title</h1><p>Hello &amp; welcome.</p><p>Goodbye.</p>";
        let result = super::normalize(html, SourceType::Html);
        assert!(result.contains("Title"));
        assert!(result.contains("Hello & welcome."));
        assert!(result.contains("Goodbye."));
        assert!(!result.contains("<h1>"));
        assert!(!result.contains("<p>"));
    }

    #[test]
    fn normalize_markdown_strips_formatting() {
        let md = "# Title\n\nSome **bold** and *italic* text.\n\n[link](http://example.com)\n\n```\ncode block\n```";
        let result = super::normalize(md, SourceType::Markdown);
        assert!(result.contains("Title"));
        assert!(result.contains("bold"));
        assert!(result.contains("italic"));
        assert!(result.contains("link"));
        assert!(!result.contains("**"));
        assert!(!result.contains("http://example.com"));
        assert!(!result.contains("```"));
    }

    #[test]
    fn normalize_json_extracts_text_values() {
        let json =
            r#"{"title": "My Doc", "items": [{"name": "Item A"}, {"name": "Item B"}], "count": 2}"#;
        let result = super::normalize(json, SourceType::StructuredJson);
        assert!(result.contains("My Doc"));
        assert!(result.contains("Item A"));
        assert!(result.contains("Item B"));
    }

    #[test]
    fn normalize_invalid_json_falls_back_to_raw() {
        let bad = "not valid json {{{";
        let result = super::normalize(bad, SourceType::StructuredJson);
        assert_eq!(result, bad);
    }

    #[test]
    fn normalize_html_unclosed_tag_does_not_panic() {
        let html = "Hello <b";
        let result = super::normalize(html, SourceType::Html);
        // Should not panic; content before the unclosed tag is preserved.
        assert!(result.contains("Hello"));
    }

    #[test]
    fn normalize_html_unclosed_entity_does_not_panic() {
        let html = "Hello &unknown stuff";
        let result = super::normalize(html, SourceType::Html);
        assert!(result.contains("Hello"));
    }

    #[test]
    fn normalize_empty_string_returns_empty() {
        assert_eq!(super::normalize("", SourceType::PlainText), "");
        assert_eq!(super::normalize("", SourceType::Html), "");
        assert_eq!(super::normalize("", SourceType::Markdown), "");
        assert_eq!(super::normalize("", SourceType::StructuredJson), "");
    }

    #[test]
    fn normalize_markdown_unclosed_bracket_does_not_panic() {
        let md = "Check [this link without closing";
        let result = super::normalize(md, SourceType::Markdown);
        assert!(result.contains("this link without closing"));
    }

    #[tokio::test]
    async fn ingest_html_normalizes_before_chunking() {
        let store = MemDocStore::new();
        let chunker = ParagraphChunker {
            max_chunk_size: 500,
        };
        let pipeline = IngestPipeline::new(store, chunker);

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new("doc_html"),
                source_id: SourceId::new("src"),
                source_type: SourceType::Html,
                project: ProjectKey::new("t", "w", "p"),
                content: "<h1>Guide</h1><p>Step one: &amp; do this.</p>".to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let chunks = pipeline.store.chunks.lock().unwrap();
        assert!(!chunks.is_empty());
        let text = &chunks[0].text;
        assert!(text.contains("Guide"));
        assert!(text.contains("Step one: & do this."));
        assert!(!text.contains("<h1>"));
    }
}
