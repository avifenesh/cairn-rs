//! Research + Digest service boundaries (GAP-016).

use async_trait::async_trait;
use cairn_domain::research::{DigestReport, ResearchQuery, ResearchResult};

use crate::error::RuntimeError;

/// Service for running research queries through the LLM-curated pipeline.
#[async_trait]
pub trait ResearchService: Send + Sync {
    /// Execute a research query and return a curated result.
    async fn run_query(&self, q: ResearchQuery) -> Result<ResearchResult, RuntimeError>;

    /// List recent research results, most-recent first.
    async fn list_results(&self, limit: usize) -> Result<Vec<ResearchResult>, RuntimeError>;
}

/// Service for generating and retrieving digest reports.
#[async_trait]
pub trait DigestService: Send + Sync {
    /// Generate a digest report covering the given time window.
    ///
    /// `period_ms` is the window length in milliseconds (e.g. 7 days = 604_800_000).
    async fn generate_digest(
        &self,
        title: String,
        period_ms: u64,
    ) -> Result<DigestReport, RuntimeError>;

    /// List recent digest reports, most-recent first.
    async fn list_digests(&self, limit: usize) -> Result<Vec<DigestReport>, RuntimeError>;
}
