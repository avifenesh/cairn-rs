//! GAP-016: In-memory stub implementations of ResearchService and DigestService.
//!
//! These stubs:
//! - Echo the query prompt as the result summary (deterministic, testable).
//! - Return empty-entry digest reports (no LLM call required).
//! - Persist results/reports in-process for `list_*` queries.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cairn_domain::research::{DigestEntry, DigestReport, ResearchQuery, ResearchResult};

use crate::error::RuntimeError;
use crate::research::{DigestService, ResearchService};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── InMemoryResearchService ───────────────────────────────────────────────────

/// Stub research service. Returns the query prompt echoed as the summary.
pub struct InMemoryResearchService {
    results: Mutex<Vec<ResearchResult>>,
}

impl InMemoryResearchService {
    pub fn new() -> Self {
        Self {
            results: Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryResearchService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ResearchService for InMemoryResearchService {
    async fn run_query(&self, q: ResearchQuery) -> Result<ResearchResult, RuntimeError> {
        let result = ResearchResult {
            query_id: q.query_id.clone(),
            // Stub: echo the prompt as the summary so callers can assert on it.
            summary: format!("[stub] {}", q.prompt),
            sources_used: q.sources.clone(),
            confidence: 0.5,
            created_at_ms: now_ms(),
        };
        self.results
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(result.clone());
        Ok(result)
    }

    async fn list_results(&self, limit: usize) -> Result<Vec<ResearchResult>, RuntimeError> {
        let results = self.results.lock().unwrap_or_else(|e| e.into_inner());
        // Most-recent first.
        let mut v: Vec<ResearchResult> = results.iter().rev().take(limit).cloned().collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms));
        Ok(v)
    }
}

// ── InMemoryDigestService ─────────────────────────────────────────────────────

/// Stub digest service. Returns empty-entries reports with correct metadata.
pub struct InMemoryDigestService {
    reports: Mutex<Vec<DigestReport>>,
}

impl InMemoryDigestService {
    pub fn new() -> Self {
        Self {
            reports: Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryDigestService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DigestService for InMemoryDigestService {
    async fn generate_digest(
        &self,
        title: String,
        period_ms: u64,
    ) -> Result<DigestReport, RuntimeError> {
        let now = now_ms();
        let report = DigestReport {
            report_id: format!("digest_{now}"),
            title,
            period_start_ms: now.saturating_sub(period_ms),
            period_end_ms: now,
            // Stub: no entries (real implementation would query the knowledge graph).
            entries: Vec::<DigestEntry>::new(),
            generated_at_ms: now,
        };
        self.reports
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(report.clone());
        Ok(report)
    }

    async fn list_digests(&self, limit: usize) -> Result<Vec<DigestReport>, RuntimeError> {
        let reports = self.reports.lock().unwrap_or_else(|e| e.into_inner());
        Ok(reports.iter().rev().take(limit).cloned().collect())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::research::ResearchQuery;

    fn query(id: &str, prompt: &str) -> ResearchQuery {
        ResearchQuery {
            query_id: id.to_owned(),
            prompt: prompt.to_owned(),
            sources: vec!["src_1".to_owned()],
            max_results: 5,
            created_at_ms: 1_000,
        }
    }

    // ── ResearchService tests ─────────────────────────────────────────────────

    /// run_query echoes prompt in the summary and links back to query_id.
    #[tokio::test]
    async fn run_query_returns_result_with_echoed_summary() {
        let svc = InMemoryResearchService::new();
        let result = svc
            .run_query(query("q1", "latest Rust news"))
            .await
            .unwrap();

        assert_eq!(result.query_id, "q1");
        assert!(
            result.summary.contains("latest Rust news"),
            "stub must echo the prompt in the summary"
        );
        assert_eq!(result.sources_used, vec!["src_1"]);
        assert!(result.confidence > 0.0 && result.confidence <= 1.0);
    }

    /// list_results returns previously run results, bounded by limit.
    #[tokio::test]
    async fn list_results_returns_stored_results() {
        let svc = InMemoryResearchService::new();
        svc.run_query(query("q1", "topic A")).await.unwrap();
        svc.run_query(query("q2", "topic B")).await.unwrap();

        let all = svc.list_results(10).await.unwrap();
        assert_eq!(all.len(), 2);

        let limited = svc.list_results(1).await.unwrap();
        assert_eq!(limited.len(), 1);
    }

    /// list_results with limit=0 returns empty vec.
    #[tokio::test]
    async fn list_results_zero_limit_returns_empty() {
        let svc = InMemoryResearchService::new();
        svc.run_query(query("q1", "x")).await.unwrap();
        let empty = svc.list_results(0).await.unwrap();
        assert!(empty.is_empty());
    }

    // ── DigestService tests ───────────────────────────────────────────────────

    /// generate_digest returns a report with valid metadata and empty entries.
    #[tokio::test]
    async fn generate_digest_returns_report_with_correct_metadata() {
        let svc = InMemoryDigestService::new();
        let period_7d_ms: u64 = 7 * 24 * 3_600_000;

        let report = svc
            .generate_digest("Weekly Tech Digest".to_owned(), period_7d_ms)
            .await
            .unwrap();

        assert_eq!(report.title, "Weekly Tech Digest");
        assert!(!report.report_id.is_empty());
        assert!(
            report.is_valid_period(),
            "period_end must be > period_start"
        );
        assert_eq!(
            report.period_end_ms - report.period_start_ms,
            period_7d_ms,
            "period length must match requested duration"
        );
        // Stub returns no entries.
        assert!(report.entries.is_empty(), "stub must return empty entries");
    }

    /// list_digests returns generated reports bounded by limit.
    #[tokio::test]
    async fn list_digests_returns_stored_reports() {
        let svc = InMemoryDigestService::new();
        svc.generate_digest("Digest A".to_owned(), 86_400_000)
            .await
            .unwrap();
        svc.generate_digest("Digest B".to_owned(), 86_400_000)
            .await
            .unwrap();

        let all = svc.list_digests(10).await.unwrap();
        assert_eq!(all.len(), 2);

        let limited = svc.list_digests(1).await.unwrap();
        assert_eq!(limited.len(), 1);
    }

    /// list_digests when no reports exist returns empty vec (no panic).
    #[tokio::test]
    async fn list_digests_empty_returns_empty_vec() {
        let svc = InMemoryDigestService::new();
        let result = svc.list_digests(5).await.unwrap();
        assert!(result.is_empty());
    }

    /// run_query with empty sources preserves empty sources_used in result.
    #[tokio::test]
    async fn run_query_with_no_sources_returns_empty_sources_used() {
        let svc = InMemoryResearchService::new();
        let q = ResearchQuery {
            query_id: "q_empty".to_owned(),
            prompt: "anything".to_owned(),
            sources: vec![],
            max_results: 3,
            created_at_ms: 0,
        };
        let result = svc.run_query(q).await.unwrap();
        assert!(
            result.sources_used.is_empty(),
            "empty sources in → empty sources_used out"
        );
    }

    /// Digests generated with different periods produce non-overlapping windows.
    #[tokio::test]
    async fn digest_period_reflects_requested_duration() {
        let svc = InMemoryDigestService::new();
        let one_day_ms: u64 = 86_400_000;

        let report = svc
            .generate_digest("Daily".to_owned(), one_day_ms)
            .await
            .unwrap();
        let window = report.period_end_ms - report.period_start_ms;
        assert_eq!(window, one_day_ms);
    }
}
