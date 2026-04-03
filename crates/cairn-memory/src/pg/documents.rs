use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};

use crate::ingest::{ChunkRecord, IngestError, IngestStatus, SourceType};

/// Postgres-backed document and chunk persistence.
///
/// Stores documents and chunks in the shared cairn-store schema
/// (V010/V011 migrations).
pub struct PgDocumentStore {
    pool: PgPool,
}

impl PgDocumentStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert a document record.
    pub async fn insert_document(
        &self,
        document_id: &KnowledgeDocumentId,
        source_id: &SourceId,
        source_type: SourceType,
        project: &ProjectKey,
        title: Option<&str>,
    ) -> Result<(), IngestError> {
        let now = now_ms();
        let source_type_str = source_type_str(source_type);

        sqlx::query(
            "INSERT INTO documents (document_id, source_id, tenant_id, workspace_id, project_id, source_type, title, ingest_status, version, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', 1, $8, $8)",
        )
        .bind(document_id.as_str())
        .bind(source_id.as_str())
        .bind(project.tenant_id.as_str())
        .bind(project.workspace_id.as_str())
        .bind(project.project_id.as_str())
        .bind(source_type_str)
        .bind(title)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| IngestError::StorageError(e.to_string()))?;

        Ok(())
    }

    /// Update ingest status for a document.
    pub async fn update_status(
        &self,
        document_id: &KnowledgeDocumentId,
        status: IngestStatus,
    ) -> Result<(), IngestError> {
        let now = now_ms();
        let status_str = ingest_status_str(status);

        sqlx::query(
            "UPDATE documents SET ingest_status = $1, version = version + 1, updated_at = $2 WHERE document_id = $3",
        )
        .bind(status_str)
        .bind(now)
        .bind(document_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| IngestError::StorageError(e.to_string()))?;

        Ok(())
    }

    /// Insert chunks for a document.
    pub async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> Result<(), IngestError> {
        if chunks.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| IngestError::StorageError(e.to_string()))?;

        for chunk in chunks {
            let source_type_str = source_type_str(chunk.source_type);

            sqlx::query(
                "INSERT INTO chunks (chunk_id, document_id, source_id, tenant_id, workspace_id, project_id, source_type, text, position, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(chunk.chunk_id.as_str())
            .bind(chunk.document_id.as_str())
            .bind(chunk.source_id.as_str())
            .bind(chunk.project.tenant_id.as_str())
            .bind(chunk.project.workspace_id.as_str())
            .bind(chunk.project.project_id.as_str())
            .bind(source_type_str)
            .bind(&chunk.text)
            .bind(chunk.position as i32)
            .bind(chunk.created_at as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| IngestError::StorageError(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| IngestError::StorageError(e.to_string()))?;

        Ok(())
    }

    /// Get ingest status for a document.
    pub async fn get_status(
        &self,
        document_id: &KnowledgeDocumentId,
    ) -> Result<Option<IngestStatus>, IngestError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT ingest_status FROM documents WHERE document_id = $1")
                .bind(document_id.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| IngestError::StorageError(e.to_string()))?;

        Ok(row.and_then(|(s,)| parse_ingest_status(&s)))
    }

    /// List chunks for a document.
    pub async fn list_chunks(
        &self,
        document_id: &KnowledgeDocumentId,
    ) -> Result<Vec<ChunkRecord>, IngestError> {
        let rows = sqlx::query_as::<_, ChunkRow>(
            "SELECT chunk_id, document_id, source_id, tenant_id, workspace_id, project_id, source_type, text, position, created_at
             FROM chunks WHERE document_id = $1 ORDER BY position",
        )
        .bind(document_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| IngestError::StorageError(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_chunk_record()).collect())
    }
}

#[derive(sqlx::FromRow)]
struct ChunkRow {
    chunk_id: String,
    document_id: String,
    source_id: String,
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    source_type: String,
    text: String,
    position: i32,
    created_at: i64,
}

impl ChunkRow {
    fn into_chunk_record(self) -> ChunkRecord {
        ChunkRecord {
            chunk_id: ChunkId::new(self.chunk_id),
            document_id: KnowledgeDocumentId::new(self.document_id),
            source_id: SourceId::new(self.source_id),
            source_type: parse_source_type(&self.source_type).unwrap_or(SourceType::PlainText),
            project: ProjectKey::new(self.tenant_id, self.workspace_id, self.project_id),
            text: self.text,
            position: self.position as u32,
            created_at: self.created_at as u64,
            updated_at: None,
            provenance_metadata: None,
            credibility_score: None,
            graph_linkage: None,
            embedding: None,
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn source_type_str(st: SourceType) -> &'static str {
    match st {
        SourceType::PlainText => "plain_text",
        SourceType::Markdown => "markdown",
        SourceType::Html => "html",
        SourceType::StructuredJson => "structured_json",
        SourceType::KnowledgePack => "knowledge_pack",
    }
}

fn parse_source_type(s: &str) -> Option<SourceType> {
    match s {
        "plain_text" => Some(SourceType::PlainText),
        "markdown" => Some(SourceType::Markdown),
        "html" => Some(SourceType::Html),
        "structured_json" => Some(SourceType::StructuredJson),
        "knowledge_pack" => Some(SourceType::KnowledgePack),
        _ => None,
    }
}

fn ingest_status_str(status: IngestStatus) -> &'static str {
    match status {
        IngestStatus::Pending => "pending",
        IngestStatus::Parsing => "parsing",
        IngestStatus::Chunking => "chunking",
        IngestStatus::Embedding => "embedding",
        IngestStatus::Indexing => "indexing",
        IngestStatus::Completed => "completed",
        IngestStatus::Failed => "failed",
    }
}

fn parse_ingest_status(s: &str) -> Option<IngestStatus> {
    match s {
        "pending" => Some(IngestStatus::Pending),
        "parsing" => Some(IngestStatus::Parsing),
        "chunking" => Some(IngestStatus::Chunking),
        "embedding" => Some(IngestStatus::Embedding),
        "indexing" => Some(IngestStatus::Indexing),
        "completed" => Some(IngestStatus::Completed),
        "failed" => Some(IngestStatus::Failed),
        _ => None,
    }
}
