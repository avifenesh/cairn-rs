use async_trait::async_trait;
use sqlx::PgPool;
use std::time::SystemTime;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};

use crate::ingest::{ChunkRecord, SourceType};
use crate::retrieval::{
    RetrievalDiagnostics, RetrievalError, RetrievalMode, RetrievalQuery, RetrievalResponse,
    RetrievalResult, RetrievalService, ScoringBreakdown,
};

/// Postgres-backed retrieval service using full-text search.
///
/// Per RFC 003, the v1 lexical floor uses Postgres FTS plus
/// product-owned normalization and filtering. Vector retrieval
/// will be added when pgvector is wired in.
pub struct PgRetrievalService {
    pool: PgPool,
}

impl PgRetrievalService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl RetrievalService for PgRetrievalService {
    async fn query(&self, query: RetrievalQuery) -> Result<RetrievalResponse, RetrievalError> {
        let start = SystemTime::now();

        // Mode honesty: VectorOnly requires pgvector (not wired yet).
        // Hybrid explicitly falls back to lexical and reports it.
        let effective_mode = match query.mode {
            RetrievalMode::VectorOnly => {
                return Err(RetrievalError::Internal(
                    "VectorOnly requires pgvector which is not yet wired. \
                     Use LexicalOnly or Hybrid (falls back to lexical)."
                        .to_owned(),
                ));
            }
            RetrievalMode::Hybrid => RetrievalMode::LexicalOnly,
            other => other,
        };

        let results = self.lexical_search(&query).await?;

        let elapsed = start.elapsed().unwrap_or_default().as_millis() as u64;

        let diagnostics = RetrievalDiagnostics {
            mode_used: effective_mode,
            reranker_used: query.reranker,
            candidates_generated: results.len(),
            results_returned: results.len(),
            latency_ms: elapsed,
        };

        Ok(RetrievalResponse {
            results,
            diagnostics,
        })
    }
}

impl PgRetrievalService {
    async fn lexical_search(
        &self,
        query: &RetrievalQuery,
    ) -> Result<Vec<RetrievalResult>, RetrievalError> {
        let rows = sqlx::query_as::<_, ChunkSearchRow>(
            "SELECT c.chunk_id, c.document_id, c.source_id, c.tenant_id, c.workspace_id, c.project_id,
                    c.source_type, c.text, c.position, c.created_at,
                    ts_rank(c.tsv, plainto_tsquery('english', $1)) AS rank
             FROM chunks c
             WHERE c.tenant_id = $2
               AND c.workspace_id = $3
               AND c.project_id = $4
               AND c.tsv @@ plainto_tsquery('english', $1)
             ORDER BY rank DESC
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
    rank: f32,
}

impl ChunkSearchRow {
    fn into_result(self) -> RetrievalResult {
        let chunk = ChunkRecord {
            chunk_id: self.chunk_id,
            document_id: KnowledgeDocumentId::new(self.document_id),
            source_id: SourceId::new(self.source_id),
            source_type: parse_source_type(&self.source_type).unwrap_or(SourceType::PlainText),
            project: ProjectKey::new(self.tenant_id, self.workspace_id, self.project_id),
            text: self.text,
            position: self.position as u32,
            created_at: self.created_at as u64,
        };

        RetrievalResult {
            chunk,
            score: self.rank as f64,
            breakdown: ScoringBreakdown {
                lexical_relevance: self.rank as f64,
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
