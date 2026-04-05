use std::sync::Arc;
use async_trait::async_trait;
use cairn_domain::{LlmCallTrace, SessionId};
use cairn_store::projections::LlmCallTraceReadModel;
use crate::error::RuntimeError;
use crate::observability::{LatencyStats, LlmObservabilityService};

pub struct LlmObservabilityServiceImpl<S> {
    store: Arc<S>,
}

impl<S> LlmObservabilityServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<S> LlmObservabilityService for LlmObservabilityServiceImpl<S>
where
    S: LlmCallTraceReadModel + 'static,
{
    async fn record(&self, trace: LlmCallTrace) -> Result<(), RuntimeError> {
        self.store.insert_trace(trace).await.map_err(RuntimeError::Store)
    }

    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
    ) -> Result<Vec<LlmCallTrace>, RuntimeError> {
        self.store
            .list_by_session(session_id, limit)
            .await
            .map_err(RuntimeError::Store)
    }

    async fn list_all(&self, limit: usize) -> Result<Vec<LlmCallTrace>, RuntimeError> {
        self.store
            .list_all_traces(limit)
            .await
            .map_err(RuntimeError::Store)
    }

    async fn latency_percentiles(&self, window_ms: u64) -> Result<LatencyStats, RuntimeError> {
        let traces = self.store.list_all_traces(1000).await.map_err(RuntimeError::Store)?;
        let cutoff = now_ms().saturating_sub(window_ms);

        let mut latencies: Vec<u64> = traces
            .iter()
            .filter(|t| t.created_at_ms >= cutoff)
            .map(|t| t.latency_ms)
            .collect();

        let sample_count = latencies.len() as u64;
        if sample_count == 0 {
            return Ok(LatencyStats { p50_ms: 0, p95_ms: 0, sample_count: 0 });
        }

        latencies.sort_unstable();
        let p50_idx = (sample_count as usize * 50 / 100).min(latencies.len() - 1);
        let p95_idx = (sample_count as usize * 95 / 100).min(latencies.len() - 1);

        Ok(LatencyStats {
            p50_ms: latencies[p50_idx],
            p95_ms: latencies[p95_idx],
            sample_count,
        })
    }

    async fn error_rate(&self, window_ms: u64) -> Result<f32, RuntimeError> {
        let traces = self.store.list_all_traces(1000).await.map_err(RuntimeError::Store)?;
        let cutoff = now_ms().saturating_sub(window_ms);

        let window_traces: Vec<&LlmCallTrace> = traces
            .iter()
            .filter(|t| t.created_at_ms >= cutoff)
            .collect();

        if window_traces.is_empty() {
            return Ok(0.0);
        }

        let errors = window_traces.iter().filter(|t| t.is_error).count();
        Ok(errors as f32 / window_traces.len() as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use cairn_domain::LlmCallTrace;
    use cairn_store::InMemoryStore;

    fn make_trace(trace_id: &str, latency_ms: u64, is_error: bool, created_at_ms: u64) -> LlmCallTrace {
        LlmCallTrace {
            trace_id: trace_id.to_owned(),
            model_id: "test-model".to_owned(),
            prompt_tokens: 100,
            completion_tokens: 50,
            latency_ms,
            cost_micros: 500,
            session_id: None,
            run_id: None,
            created_at_ms,
            is_error,
        }
    }

    /// Insert 10 traces with known latencies and verify p50/p95 are correct.
    ///
    /// Latencies: 10, 20, 30, 40, 50, 60, 70, 80, 90, 100 ms (sorted ascending)
    /// p50 index = 10 * 50 / 100 = 5 → latencies[5] = 60
    /// p95 index = 10 * 95 / 100 = 9 → latencies[9] = 100
    #[tokio::test]
    async fn latency_percentiles_computed_correctly() {
        let store = Arc::new(InMemoryStore::new());
        let svc = LlmObservabilityServiceImpl::new(store);

        // Use timestamps far in the future so they're always within any window.
        let far_future = u64::MAX / 2;

        for i in 1..=10u64 {
            svc.record(make_trace(
                &format!("t{i}"),
                i * 10,      // 10, 20, ..., 100 ms
                false,
                far_future + i,
            )).await.unwrap();
        }

        // Use a huge window to include all traces.
        let stats = svc.latency_percentiles(u64::MAX / 2).await.unwrap();
        assert_eq!(stats.sample_count, 10, "must have 10 samples");
        // sorted: [10,20,30,40,50,60,70,80,90,100]
        // p50 idx = 5 → 60
        assert_eq!(stats.p50_ms, 60, "p50 must be 60 ms");
        // p95 idx = 9 → 100
        assert_eq!(stats.p95_ms, 100, "p95 must be 100 ms");
    }

    #[tokio::test]
    async fn latency_percentiles_empty_returns_zeros() {
        let store = Arc::new(InMemoryStore::new());
        let svc = LlmObservabilityServiceImpl::new(store);
        let stats = svc.latency_percentiles(60_000).await.unwrap();
        assert_eq!(stats.sample_count, 0);
        assert_eq!(stats.p50_ms, 0);
        assert_eq!(stats.p95_ms, 0);
    }

    #[tokio::test]
    async fn error_rate_computed_correctly() {
        let store = Arc::new(InMemoryStore::new());
        let svc = LlmObservabilityServiceImpl::new(store);
        let far_future = u64::MAX / 2;

        // 7 successes, 3 errors → 0.3
        for i in 0u64..7 {
            svc.record(make_trace(&format!("ok_{i}"), 50, false, far_future + i)).await.unwrap();
        }
        for i in 0u64..3 {
            svc.record(make_trace(&format!("err_{i}"), 50, true, far_future + 7 + i)).await.unwrap();
        }

        let rate = svc.error_rate(u64::MAX / 2).await.unwrap();
        assert!(
            (rate - 0.3).abs() < 0.001,
            "error rate must be 0.3, got {rate}"
        );
    }

    #[tokio::test]
    async fn error_rate_zero_when_no_traces() {
        let store = Arc::new(InMemoryStore::new());
        let svc = LlmObservabilityServiceImpl::new(store);
        let rate = svc.error_rate(60_000).await.unwrap();
        assert_eq!(rate, 0.0);
    }

    #[tokio::test]
    async fn latency_percentiles_single_sample() {
        let store = Arc::new(InMemoryStore::new());
        let svc = LlmObservabilityServiceImpl::new(store);
        let far_future = u64::MAX / 2;

        svc.record(make_trace("only", 42, false, far_future)).await.unwrap();
        let stats = svc.latency_percentiles(u64::MAX / 2).await.unwrap();
        assert_eq!(stats.sample_count, 1);
        assert_eq!(stats.p50_ms, 42);
        assert_eq!(stats.p95_ms, 42);
    }
}
