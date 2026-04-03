use async_trait::async_trait;
use sqlx::SqlitePool;
use std::time::Instant;

use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};

use crate::ingest::{ChunkRecord, SourceType};
use crate::retrieval::{
    RetrievalDiagnostics, RetrievalError, RetrievalMode, RetrievalQuery, RetrievalResponse,
    RetrievalResult, RetrievalService, ScoringBreakdown,
};

/// SQLite-backed retrieval using FTS5 for local-mode.
///
/// Per RFC 003, local mode provides FTS-backed lexical retrieval
/// where feasible, with no HNSW/pgvector requirement.
pub struct SqliteRetrievalService {
    pool: SqlitePool,
}

impl SqliteRetrievalService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl RetrievalService for SqliteRetrievalService {
    async fn query(&self, query: RetrievalQuery) -> Result<RetrievalResponse, RetrievalError> {
        let start = Instant::now();

        let results = match query.mode {
            RetrievalMode::LexicalOnly | RetrievalMode::Hybrid => self.fts5_search(&query).await?,
            RetrievalMode::VectorOnly => {
                // Vector search not available in SQLite local-mode.
                // Return empty with degraded diagnostics.
                vec![]
            }
        };

        let elapsed = start.elapsed().as_millis() as u64;

        Ok(RetrievalResponse {
            results: results.clone(),
            diagnostics: RetrievalDiagnostics {
                mode_used: query.mode,
                reranker_used: query.reranker,
                candidates_generated: results.len(),
                results_returned: results.len(),
                latency_ms: elapsed,
            },
        })
    }
}

impl SqliteRetrievalService {
    async fn fts5_search(
        &self,
        query: &RetrievalQuery,
    ) -> Result<Vec<RetrievalResult>, RetrievalError> {
        let rows = sqlx::query_as::<_, ChunkSearchRow>(
            "SELECT c.chunk_id, c.document_id, c.source_id, c.tenant_id, c.workspace_id,
                    c.project_id, c.source_type, c.text, c.position, c.created_at,
                    bm25(chunks_fts) AS rank
             FROM chunks_fts f
             JOIN chunks c ON c.chunk_id = f.chunk_id
             WHERE chunks_fts MATCH $1
               AND c.tenant_id = $2
               AND c.workspace_id = $3
               AND c.project_id = $4
             ORDER BY rank
             LIMIT $5",
        )
        .bind(&query.query_text)
        .bind(query.project.tenant_id.as_str())
        .bind(query.project.workspace_id.as_str())
        .bind(query.project.project_id.as_str())
        .bind(query.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RetrievalError::StorageError(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_result()).collect())
    }
}

#[derive(sqlx::FromRow)]
struct ChunkSearchRow {
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
    rank: f64,
}

impl ChunkSearchRow {
    fn into_result(self) -> RetrievalResult {
        let chunk = ChunkRecord {
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
        };

        // bm25() returns negative values (more negative = better match).
        let score = (-self.rank).max(0.0);

        RetrievalResult {
            chunk,
            score,
            breakdown: ScoringBreakdown {
                lexical_relevance: score,
                ..ScoringBreakdown::default()
            },
        }
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
