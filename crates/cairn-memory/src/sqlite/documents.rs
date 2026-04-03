use async_trait::async_trait;
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};

use crate::ingest::{ChunkRecord, IngestError, IngestStatus, SourceType};
use crate::pipeline::DocumentStore;

/// SQLite-backed document and chunk persistence for local-mode.
pub struct SqliteDocumentStore {
    pool: SqlitePool,
}

impl SqliteDocumentStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DocumentStore for SqliteDocumentStore {
    async fn insert_document(
        &self,
        document_id: &KnowledgeDocumentId,
        source_id: &SourceId,
        source_type: SourceType,
        project: &ProjectKey,
        title: Option<&str>,
    ) -> Result<(), IngestError> {
        let now = now_ms();
        let st = source_type_str(source_type);

        sqlx::query(
            "INSERT INTO documents (document_id, source_id, tenant_id, workspace_id, project_id, source_type, title, ingest_status, version, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', 1, $8, $8)",
        )
        .bind(document_id.as_str())
        .bind(source_id.as_str())
        .bind(project.tenant_id.as_str())
        .bind(project.workspace_id.as_str())
        .bind(project.project_id.as_str())
        .bind(st)
        .bind(title)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| IngestError::StorageError(e.to_string()))?;

        Ok(())
    }

    async fn update_status(
        &self,
        document_id: &KnowledgeDocumentId,
        status: IngestStatus,
    ) -> Result<(), IngestError> {
        let now = now_ms();
        let s = ingest_status_str(status);

        sqlx::query(
            "UPDATE documents SET ingest_status = $1, version = version + 1, updated_at = $2 WHERE document_id = $3",
        )
        .bind(s)
        .bind(now)
        .bind(document_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| IngestError::StorageError(e.to_string()))?;

        Ok(())
    }

    async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> Result<(), IngestError> {
        for chunk in chunks {
            let st = source_type_str(chunk.source_type);

            sqlx::query(
                "INSERT INTO chunks (chunk_id, document_id, source_id, tenant_id, workspace_id, project_id, source_type, text, position, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(&chunk.chunk_id)
            .bind(chunk.document_id.as_str())
            .bind(chunk.source_id.as_str())
            .bind(chunk.project.tenant_id.as_str())
            .bind(chunk.project.workspace_id.as_str())
            .bind(chunk.project.project_id.as_str())
            .bind(st)
            .bind(&chunk.text)
            .bind(chunk.position as i32)
            .bind(chunk.created_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| IngestError::StorageError(e.to_string()))?;

            // Sync FTS5 index (no triggers due to migration runner limitation).
            sqlx::query("INSERT INTO chunks_fts (chunk_id, text) VALUES ($1, $2)")
                .bind(&chunk.chunk_id)
                .bind(&chunk.text)
                .execute(&self.pool)
                .await
                .map_err(|e| IngestError::StorageError(e.to_string()))?;
        }

        Ok(())
    }

    async fn get_status(
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

fn ingest_status_str(s: IngestStatus) -> &'static str {
    match s {
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
