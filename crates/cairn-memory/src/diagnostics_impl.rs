use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

use cairn_domain::{ProjectKey, SourceId};

use crate::diagnostics::{DiagnosticsError, DiagnosticsService, IndexStatus, SourceQualityRecord};

/// In-memory diagnostics service that tracks source quality and index status.
///
/// Per RFC 003, the product must expose ingest status, embedding/index status,
/// source quality views, and retrieval diagnostics.
pub struct InMemoryDiagnostics {
    sources: Mutex<HashMap<String, SourceQualityRecord>>,
    index_status: Mutex<HashMap<String, IndexStatus>>,
}

impl InMemoryDiagnostics {
    pub fn new() -> Self {
        Self {
            sources: Mutex::new(HashMap::new()),
            index_status: Mutex::new(HashMap::new()),
        }
    }

    /// Record a document ingest for diagnostics tracking.
    pub fn record_ingest(&self, source_id: &SourceId, project: &ProjectKey, chunk_count: u64) {
        let key = source_id.as_str().to_owned();
        let mut sources = self.sources.lock().unwrap();
        let entry = sources.entry(key).or_insert_with(|| SourceQualityRecord {
            source_id: source_id.clone(),
            project: project.clone(),
            total_chunks: 0,
            total_retrievals: 0,
            avg_relevance_score: 0.0,
            freshness_score: 1.0,
            credibility_score: 1.0,
            last_ingested_at: 0,
            avg_rating: 0.0,
            retrieval_count: 0,
            query_hit_rate: 0.0,
            error_rate: 0.0,
        });
        entry.total_chunks += chunk_count;
        entry.last_ingested_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Update index status.
        let proj_key = format!(
            "{}:{}:{}",
            project.tenant_id.as_str(),
            project.workspace_id.as_str(),
            project.project_id.as_str()
        );
        let mut idx = self.index_status.lock().unwrap();
        let status = idx.entry(proj_key).or_insert_with(|| IndexStatus {
            project: project.clone(),
            total_documents: 0,
            total_chunks: 0,
            pending_embeddings: 0,
            stale_chunks: 0,
        });
        status.total_documents += 1;
        status.total_chunks += chunk_count;
    }

    /// Record a retrieval hit for a source (updates quality metrics).
    pub fn record_retrieval_hit(&self, source_id: &SourceId, relevance_score: f64) {
        let key = source_id.as_str().to_owned();
        let mut sources = self.sources.lock().unwrap();
        if let Some(entry) = sources.get_mut(&key) {
            let n = entry.total_retrievals as f64;
            entry.avg_relevance_score =
                (entry.avg_relevance_score * n + relevance_score) / (n + 1.0);
            entry.total_retrievals += 1;
        }
    }
}

impl Default for InMemoryDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DiagnosticsService for InMemoryDiagnostics {
    async fn source_quality(
        &self,
        source_id: &SourceId,
    ) -> Result<Option<SourceQualityRecord>, DiagnosticsError> {
        let sources = self.sources.lock().unwrap();
        Ok(sources.get(source_id.as_str()).cloned())
    }

    async fn list_source_quality(
        &self,
        project: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<SourceQualityRecord>, DiagnosticsError> {
        let sources = self.sources.lock().unwrap();
        let mut results: Vec<SourceQualityRecord> = sources
            .values()
            .filter(|s| s.project == *project)
            .cloned()
            .collect();
        results.sort_by(|a, b| {
            b.avg_relevance_score
                .partial_cmp(&a.avg_relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    async fn index_status(&self, project: &ProjectKey) -> Result<IndexStatus, DiagnosticsError> {
        let proj_key = format!(
            "{}:{}:{}",
            project.tenant_id.as_str(),
            project.workspace_id.as_str(),
            project.project_id.as_str()
        );
        let idx = self.index_status.lock().unwrap();
        Ok(idx.get(&proj_key).cloned().unwrap_or(IndexStatus {
            project: project.clone(),
            total_documents: 0,
            total_chunks: 0,
            pending_embeddings: 0,
            stale_chunks: 0,
        }))
    }

}

impl InMemoryDiagnostics {
    /// Count total documents for a tenant (approximated from source quality records).
    pub fn total_documents_for_tenant(&self, tenant_id: &cairn_domain::TenantId) -> u32 {
        let sources = self.sources.lock().unwrap();
        sources.values()
            .filter(|v| v.project.tenant_id == *tenant_id)
            .count() as u32
    }

    /// Record retrieval feedback (rating for a specific chunk).
    ///
    /// Updates `avg_rating` via a running average when a numeric rating is
    /// provided, and records a retrieval hit at the given relevance score
    /// (defaulting to 0.7 when no rating is given).
    pub fn record_retrieval_feedback(
        &self,
        source_id: &cairn_domain::SourceId,
        _chunk_id: &str,
        was_used: bool,
        rating: Option<f64>,
    ) {
        let relevance = rating.unwrap_or(0.7);
        if was_used {
            self.record_retrieval_hit(source_id, relevance);
        }
        if let Some(r) = rating {
            let key = source_id.as_str().to_owned();
            let mut sources = self.sources.lock().unwrap();
            if let Some(entry) = sources.get_mut(&key) {
                let n = entry.retrieval_count as f64;
                entry.avg_rating = (entry.avg_rating * n + r) / (n + 1.0);
                entry.retrieval_count += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn diagnostics_tracks_ingest_and_retrieval() {
        let diag = InMemoryDiagnostics::new();
        let project = ProjectKey::new("t", "w", "p");
        let source = SourceId::new("src_docs");

        diag.record_ingest(&source, &project, 5);
        diag.record_retrieval_hit(&source, 0.8);
        diag.record_retrieval_hit(&source, 0.6);

        let quality = diag.source_quality(&source).await.unwrap().unwrap();
        assert_eq!(quality.total_chunks, 5);
        assert_eq!(quality.total_retrievals, 2);
        assert!((quality.avg_relevance_score - 0.7).abs() < 0.01);

        let idx = diag.index_status(&project).await.unwrap();
        assert_eq!(idx.total_documents, 1);
        assert_eq!(idx.total_chunks, 5);
    }
}
