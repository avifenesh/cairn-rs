//! LLM observability service boundary (GAP-010).
use async_trait::async_trait;
use cairn_domain::{LlmCallTrace, SessionId};
use serde::{Deserialize, Serialize};

use crate::error::RuntimeError;

/// Percentile latency statistics over a time window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyStats {
    /// 50th-percentile (median) latency in milliseconds.
    pub p50_ms: u64,
    /// 95th-percentile latency in milliseconds.
    pub p95_ms: u64,
    /// Number of traces included in the computation.
    pub sample_count: u64,
}

/// Service for recording and querying LLM call traces.
#[async_trait]
pub trait LlmObservabilityService: Send + Sync {
    /// Record an LLM call trace.
    async fn record(&self, trace: LlmCallTrace) -> Result<(), RuntimeError>;

    /// List recent traces for a session, most-recent first.
    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
    ) -> Result<Vec<LlmCallTrace>, RuntimeError>;

    /// List all recent traces (operator view).
    async fn list_all(&self, limit: usize) -> Result<Vec<LlmCallTrace>, RuntimeError>;

    /// Compute p50/p95 latency percentiles over traces within `window_ms`.
    ///
    /// Returns `LatencyStats { p50_ms: 0, p95_ms: 0, sample_count: 0 }` when
    /// no traces fall in the window.
    async fn latency_percentiles(&self, window_ms: u64) -> Result<LatencyStats, RuntimeError>;

    /// Fraction of provider calls that failed within `window_ms` (0.0–1.0).
    ///
    /// Based on `LlmCallTrace.is_error`. Returns 0.0 when no traces exist.
    async fn error_rate(&self, window_ms: u64) -> Result<f32, RuntimeError>;
}
