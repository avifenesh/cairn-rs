use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde::{Deserialize, Serialize};

use crate::ingest::ChunkRecord;

/// Retrieval mode selection (RFC 003).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode {
    LexicalOnly,
    VectorOnly,
    Hybrid,
}

/// Reranker strategy applied after candidate generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RerankerStrategy {
    None,
    Mmr,
    ProviderReranker,
}

/// Operator-tunable weights for each scoring dimension (RFC 003).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoringWeights {
    pub semantic_weight: f64,
    pub lexical_weight: f64,
    pub freshness_weight: f64,
    pub staleness_weight: f64,
    pub credibility_weight: f64,
    pub corroboration_weight: f64,
    pub graph_proximity_weight: f64,
    pub recency_weight: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            semantic_weight: 0.4,
            lexical_weight: 0.3,
            freshness_weight: 0.1,
            staleness_weight: 0.05,
            credibility_weight: 0.05,
            corroboration_weight: 0.03,
            graph_proximity_weight: 0.05,
            recency_weight: 0.02,
        }
    }
}

/// Operator-tunable scoring policy (RFC 003).
///
/// Controls per-project or per-workspace weight presets, decay parameters,
/// and retrieval mode defaults within bounded ranges.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoringPolicy {
    pub weights: ScoringWeights,
    pub freshness_decay_days: f64,
    pub staleness_threshold_days: f64,
    pub recency_enabled: bool,
    pub retrieval_mode_default: RetrievalMode,
    pub reranker_default: RerankerStrategy,
}

impl Default for ScoringPolicy {
    fn default() -> Self {
        Self {
            weights: ScoringWeights::default(),
            freshness_decay_days: 30.0,
            staleness_threshold_days: 90.0,
            recency_enabled: false,
            retrieval_mode_default: RetrievalMode::Hybrid,
            reranker_default: RerankerStrategy::None,
        }
    }
}

/// Candidate-generation stage in the retrieval pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStage {
    Lexical,
    Vector,
    Merged,
    Reranked,
}

/// Canonical scoring dimensions (RFC 003).
///
/// All dimensions are fixed by the product contract and must be present
/// in every compliant retrieval implementation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScoringBreakdown {
    pub semantic_relevance: f64,
    pub lexical_relevance: f64,
    pub freshness: f64,
    pub staleness_penalty: f64,
    pub source_credibility: f64,
    pub corroboration: f64,
    pub graph_proximity: f64,
    pub recency_of_use: Option<f64>,
}

/// A scored retrieval result with inspectable scoring.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetrievalResult {
    pub chunk: ChunkRecord,
    pub score: f64,
    pub breakdown: ScoringBreakdown,
}

/// Retrieval query with operator-tunable parameters.
#[derive(Clone, Debug)]
pub struct RetrievalQuery {
    pub project: ProjectKey,
    pub query_text: String,
    pub mode: RetrievalMode,
    pub reranker: RerankerStrategy,
    pub limit: usize,
    pub metadata_filters: Vec<MetadataFilter>,
    pub scoring_policy: Option<ScoringPolicy>,
}

/// Simple metadata filter for retrieval queries.
#[derive(Clone, Debug)]
pub struct MetadataFilter {
    pub key: String,
    pub value: String,
}

/// Retrieval diagnostics for a completed query (RFC 003 requirement).
///
/// For every retrieval request, the product must expose: retrieval mode,
/// candidate-generation stages, scoring dimensions that contributed,
/// effective scoring policy, and reranker path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetrievalDiagnostics {
    pub mode_used: RetrievalMode,
    pub reranker_used: RerankerStrategy,
    pub candidates_generated: usize,
    pub results_returned: usize,
    pub latency_ms: u64,
    pub stages_used: Vec<CandidateStage>,
    pub scoring_dimensions_used: Vec<String>,
    pub effective_policy: Option<String>,
}

/// A retrieval response including results and diagnostics.
#[derive(Clone, Debug)]
pub struct RetrievalResponse {
    pub results: Vec<RetrievalResult>,
    pub diagnostics: RetrievalDiagnostics,
}

/// Retrieval service boundary.
///
/// Per RFC 003, retrieval runs in-process with the main runtime/API and
/// supports lexical, vector, and hybrid modes with metadata filtering,
/// reranking, and inspectable scoring.
#[async_trait]
pub trait RetrievalService: Send + Sync {
    /// Execute a retrieval query and return scored results with diagnostics.
    async fn query(&self, query: RetrievalQuery) -> Result<RetrievalResponse, RetrievalError>;
}

// --- Scoring calculators (RFC 003) ---

const MS_PER_DAY: f64 = 86_400_000.0;

/// Compute freshness score based on content age.
///
/// Returns a value in [0.0, 1.0] that decays exponentially from 1.0
/// as the content ages. `decay_days` controls the half-life-like rate:
/// content `decay_days` old scores ~0.37 (1/e).
pub fn freshness_score(created_at_ms: u64, now_ms: u64, decay_days: f64) -> f64 {
    if decay_days <= 0.0 || now_ms <= created_at_ms {
        return 1.0;
    }
    let age_days = (now_ms - created_at_ms) as f64 / MS_PER_DAY;
    (-age_days / decay_days).exp()
}

/// Compute staleness penalty for content that hasn't been updated recently.
///
/// Returns a value in [0.0, 1.0] where 0.0 means no penalty (fresh or
/// recently updated) and 1.0 means maximally stale. Content within
/// `threshold_days` of its last update receives no penalty. Beyond that,
/// the penalty grows linearly up to 1.0 at 2x the threshold.
pub fn staleness_penalty(
    updated_at_ms: Option<u64>,
    created_at_ms: u64,
    now_ms: u64,
    threshold_days: f64,
) -> f64 {
    if threshold_days <= 0.0 {
        return 0.0;
    }
    let reference_ms = updated_at_ms.unwrap_or(created_at_ms);
    if now_ms <= reference_ms {
        return 0.0;
    }
    let age_days = (now_ms - reference_ms) as f64 / MS_PER_DAY;
    if age_days <= threshold_days {
        0.0
    } else {
        ((age_days - threshold_days) / threshold_days).min(1.0)
    }
}

/// Compute a weighted final score from a scoring breakdown and weights.
///
/// The staleness_penalty dimension is subtracted (it's a penalty), all
/// others are added. recency_of_use is included only when present.
pub fn compute_final_score(breakdown: &ScoringBreakdown, weights: &ScoringWeights) -> f64 {
    let mut score = 0.0;
    score += breakdown.semantic_relevance * weights.semantic_weight;
    score += breakdown.lexical_relevance * weights.lexical_weight;
    score += breakdown.freshness * weights.freshness_weight;
    score -= breakdown.staleness_penalty * weights.staleness_weight;
    score += breakdown.source_credibility * weights.credibility_weight;
    score += breakdown.corroboration * weights.corroboration_weight;
    score += breakdown.graph_proximity * weights.graph_proximity_weight;
    if let Some(recency) = breakdown.recency_of_use {
        score += recency * weights.recency_weight;
    }
    // Guard against NaN from malformed weights/scores — treat as zero.
    if score.is_nan() {
        return 0.0;
    }
    score.max(0.0)
}

/// Retrieve current time in milliseconds (utility for scoring callers).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Retrieval-specific errors.
#[derive(Debug)]
pub enum RetrievalError {
    EmbeddingFailed(String),
    StorageError(String),
    Internal(String),
}

impl std::fmt::Display for RetrievalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RetrievalError::EmbeddingFailed(msg) => write!(f, "embedding failed: {msg}"),
            RetrievalError::StorageError(msg) => write!(f, "storage error: {msg}"),
            RetrievalError::Internal(msg) => write!(f, "internal retrieval error: {msg}"),
        }
    }
}

impl std::error::Error for RetrievalError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieval_modes_are_distinct() {
        assert_ne!(RetrievalMode::LexicalOnly, RetrievalMode::VectorOnly);
        assert_ne!(RetrievalMode::Hybrid, RetrievalMode::LexicalOnly);
    }

    #[test]
    fn default_scoring_breakdown_is_zero() {
        let b = ScoringBreakdown::default();
        assert_eq!(b.semantic_relevance, 0.0);
        assert_eq!(b.recency_of_use, None);
    }

    #[test]
    fn reranker_strategies_are_distinct() {
        assert_ne!(RerankerStrategy::None, RerankerStrategy::Mmr);
    }

    // --- Scoring calculator tests ---

    #[test]
    fn freshness_score_is_1_for_brand_new_content() {
        let now = 1_000_000;
        assert_eq!(freshness_score(now, now, 30.0), 1.0);
    }

    #[test]
    fn freshness_score_decays_over_time() {
        let day_ms = MS_PER_DAY as u64;
        let now = 100 * day_ms;
        let created_30_days_ago = now - 30 * day_ms;

        let score = freshness_score(created_30_days_ago, now, 30.0);
        // At exactly decay_days, score should be ~1/e ≈ 0.368
        assert!(score > 0.35 && score < 0.40, "score was {score}");
    }

    #[test]
    fn freshness_score_approaches_zero_for_old_content() {
        let day_ms = MS_PER_DAY as u64;
        let now = 1000 * day_ms;
        let created_long_ago = now - 365 * day_ms;

        let score = freshness_score(created_long_ago, now, 30.0);
        assert!(
            score < 0.01,
            "very old content should score near 0, got {score}"
        );
    }

    #[test]
    fn freshness_score_handles_zero_decay() {
        assert_eq!(freshness_score(0, 1_000_000, 0.0), 1.0);
    }

    #[test]
    fn staleness_penalty_zero_within_threshold() {
        let day_ms = MS_PER_DAY as u64;
        let now = 100 * day_ms;
        let updated_10_days_ago = now - 10 * day_ms;

        let penalty = staleness_penalty(Some(updated_10_days_ago), 0, now, 90.0);
        assert_eq!(penalty, 0.0);
    }

    #[test]
    fn staleness_penalty_grows_past_threshold() {
        let day_ms = MS_PER_DAY as u64;
        let now = 200 * day_ms;
        let updated_135_days_ago = now - 135 * day_ms;

        let penalty = staleness_penalty(Some(updated_135_days_ago), 0, now, 90.0);
        // 135 - 90 = 45 days past threshold. Penalty = 45/90 = 0.5
        assert!((penalty - 0.5).abs() < 0.01, "penalty was {penalty}");
    }

    #[test]
    fn staleness_penalty_caps_at_1() {
        let day_ms = MS_PER_DAY as u64;
        let now = 500 * day_ms;
        let very_old = now - 400 * day_ms;

        let penalty = staleness_penalty(Some(very_old), 0, now, 90.0);
        assert_eq!(penalty, 1.0);
    }

    #[test]
    fn staleness_uses_created_at_when_no_updated_at() {
        let day_ms = MS_PER_DAY as u64;
        let now = 200 * day_ms;
        let created_150_days_ago = now - 150 * day_ms;

        let penalty = staleness_penalty(None, created_150_days_ago, now, 90.0);
        // 150 - 90 = 60 days past. Penalty = 60/90 ≈ 0.667
        assert!(penalty > 0.6 && penalty < 0.7, "penalty was {penalty}");
    }

    #[test]
    fn compute_final_score_weighted_sum() {
        let breakdown = ScoringBreakdown {
            semantic_relevance: 0.0,
            lexical_relevance: 1.0,
            freshness: 0.8,
            staleness_penalty: 0.0,
            source_credibility: 0.0,
            corroboration: 0.0,
            graph_proximity: 0.0,
            recency_of_use: None,
        };
        let weights = ScoringWeights::default();

        let score = compute_final_score(&breakdown, &weights);
        // lexical: 1.0 * 0.3 = 0.3, freshness: 0.8 * 0.1 = 0.08 → 0.38
        assert!((score - 0.38).abs() < 0.001, "score was {score}");
    }

    #[test]
    fn compute_final_score_subtracts_staleness() {
        let breakdown = ScoringBreakdown {
            semantic_relevance: 0.0,
            lexical_relevance: 1.0,
            freshness: 0.0,
            staleness_penalty: 1.0,
            source_credibility: 0.0,
            corroboration: 0.0,
            graph_proximity: 0.0,
            recency_of_use: None,
        };
        let weights = ScoringWeights::default();

        let score = compute_final_score(&breakdown, &weights);
        // lexical: 0.3, minus staleness: 0.05 → 0.25
        assert!((score - 0.25).abs() < 0.001, "score was {score}");
    }

    #[test]
    fn compute_final_score_floors_at_zero() {
        let breakdown = ScoringBreakdown {
            staleness_penalty: 1.0,
            ..ScoringBreakdown::default()
        };
        let weights = ScoringWeights::default();

        let score = compute_final_score(&breakdown, &weights);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn compute_final_score_nan_weights_return_zero() {
        let breakdown = ScoringBreakdown {
            lexical_relevance: 1.0,
            ..ScoringBreakdown::default()
        };
        let weights = ScoringWeights {
            lexical_weight: f64::NAN,
            ..ScoringWeights::default()
        };
        assert_eq!(compute_final_score(&breakdown, &weights), 0.0);
    }

    #[test]
    fn freshness_score_future_created_at_returns_1() {
        // created_at is in the future relative to now — no decay.
        assert_eq!(freshness_score(2_000_000, 1_000_000, 30.0), 1.0);
    }

    #[test]
    fn freshness_score_huge_age_returns_near_zero() {
        // u64::MAX - 0 as age: should not panic, just return ~0.
        let score = freshness_score(0, u64::MAX, 30.0);
        assert!(score >= 0.0 && score.is_finite(), "got {score}");
    }

    #[test]
    fn staleness_penalty_negative_threshold_returns_zero() {
        assert_eq!(staleness_penalty(None, 0, 1_000_000, -10.0), 0.0);
    }

    #[test]
    fn staleness_penalty_future_reference_returns_zero() {
        // updated_at is in the future relative to now.
        assert_eq!(staleness_penalty(Some(2_000_000), 0, 1_000_000, 90.0), 0.0);
    }
}
