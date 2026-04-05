//! Research + Digest Pipeline domain types (GAP-016).
//!
//! Mirrors `cairn/internal/research/` + `internal/digest/`:
//! - **Research pipeline:** LLM-curated document retrieval with relevance scoring.
//! - **Digest engine:** Aggregates signals into a preference-aware structured summary.

use serde::{Deserialize, Serialize};

// ── Research ──────────────────────────────────────────────────────────────────

/// A research query submitted to the pipeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResearchQuery {
    /// Stable ID for this query run.
    pub query_id: String,
    /// Natural-language prompt describing what to research.
    pub prompt: String,
    /// Source IDs or URLs to draw from (empty = all available sources).
    pub sources: Vec<String>,
    /// Maximum number of documents / results to return.
    pub max_results: u32,
    pub created_at_ms: u64,
}

/// Result produced by running a `ResearchQuery` through the pipeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResearchResult {
    /// Matches the originating `ResearchQuery::query_id`.
    pub query_id: String,
    /// LLM-generated summary of the retrieved documents.
    pub summary: String,
    /// Source IDs that contributed to this result.
    pub sources_used: Vec<String>,
    /// Model-reported confidence in the summary (0.0 – 1.0).
    pub confidence: f32,
    pub created_at_ms: u64,
}

// ── Digest ────────────────────────────────────────────────────────────────────

/// A single curated item in a `DigestReport`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DigestEntry {
    pub entry_id: String,
    pub title: String,
    pub summary: String,
    /// Optional canonical URL for the source article / document.
    pub source_url: Option<String>,
    /// Relevance score (0.0 – 1.0) assigned by the curation model.
    pub relevance_score: f32,
    pub created_at_ms: u64,
}

/// A complete digest report covering a time window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DigestReport {
    pub report_id: String,
    pub title: String,
    /// Inclusive start of the coverage window (Unix ms).
    pub period_start_ms: u64,
    /// Inclusive end of the coverage window (Unix ms).
    pub period_end_ms: u64,
    pub entries: Vec<DigestEntry>,
    pub generated_at_ms: u64,
}

impl DigestReport {
    /// Returns true when the report covers a valid, non-empty time window.
    pub fn is_valid_period(&self) -> bool {
        self.period_end_ms > self.period_start_ms
    }
}

/// Schedule configuration for automatic digest generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestSchedule {
    pub schedule_id: String,
    /// Cron expression controlling generation frequency (e.g. `"0 8 * * 1"`).
    pub cron_expr: String,
    /// Research prompt used when generating each digest.
    pub query: String,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_query(id: &str) -> ResearchQuery {
        ResearchQuery {
            query_id: id.to_owned(),
            prompt: "What is new in Rust?".to_owned(),
            sources: vec!["blog.rust-lang.org".to_owned()],
            max_results: 10,
            created_at_ms: 1_000,
        }
    }

    fn sample_result(query_id: &str) -> ResearchResult {
        ResearchResult {
            query_id: query_id.to_owned(),
            summary: "Rust 2024 edition released.".to_owned(),
            sources_used: vec!["blog.rust-lang.org".to_owned()],
            confidence: 0.92,
            created_at_ms: 2_000,
        }
    }

    fn sample_entry(id: &str, score: f32) -> DigestEntry {
        DigestEntry {
            entry_id: id.to_owned(),
            title: format!("Article {id}"),
            summary: "Summary text.".to_owned(),
            source_url: Some(format!("https://example.com/{id}")),
            relevance_score: score,
            created_at_ms: 3_000,
        }
    }

    // ── Domain type tests (4 — satisfy the required 4 domain tests) ──────────

    #[test]
    fn research_query_stores_required_fields() {
        let q = sample_query("q1");
        assert_eq!(q.query_id, "q1");
        assert_eq!(q.max_results, 10);
        assert!(!q.sources.is_empty());
        assert!(q.created_at_ms > 0);
    }

    #[test]
    fn research_result_links_to_query() {
        let r = sample_result("q1");
        assert_eq!(r.query_id, "q1");
        assert!(r.confidence > 0.0 && r.confidence <= 1.0);
        assert!(!r.sources_used.is_empty());
    }

    #[test]
    fn digest_entry_carries_relevance_and_url() {
        let e = sample_entry("e1", 0.85);
        assert_eq!(e.relevance_score, 0.85);
        assert!(e.source_url.is_some());
        assert!(!e.title.is_empty());
    }

    #[test]
    fn digest_report_valid_period() {
        let report = DigestReport {
            report_id: "r1".to_owned(),
            title: "Weekly Digest".to_owned(),
            period_start_ms: 1_000,
            period_end_ms: 8_000,
            entries: vec![sample_entry("e1", 0.9)],
            generated_at_ms: 9_000,
        };
        assert!(report.is_valid_period());
        assert_eq!(report.entries.len(), 1);
    }

    #[test]
    fn digest_report_invalid_period() {
        let report = DigestReport {
            report_id: "r2".to_owned(),
            title: "Bad".to_owned(),
            period_start_ms: 5_000,
            period_end_ms: 1_000, // end before start
            entries: vec![],
            generated_at_ms: 6_000,
        };
        assert!(!report.is_valid_period());
    }

    #[test]
    fn digest_schedule_fields() {
        let sched = DigestSchedule {
            schedule_id: "sched_1".to_owned(),
            cron_expr: "0 8 * * 1".to_owned(),
            query: "weekly tech news".to_owned(),
            enabled: true,
        };
        assert!(sched.enabled);
        assert_eq!(sched.cron_expr, "0 8 * * 1");
    }

    #[test]
    fn digest_entry_optional_source_url() {
        let mut e = sample_entry("e2", 0.5);
        e.source_url = None;
        assert!(e.source_url.is_none());
    }

    #[test]
    fn research_query_empty_sources_means_all() {
        let q = ResearchQuery {
            query_id: "q_all".to_owned(),
            prompt: "broad topic".to_owned(),
            sources: vec![],
            max_results: 5,
            created_at_ms: 1_000,
        };
        assert!(q.sources.is_empty(), "empty sources = all available");
    }
}
