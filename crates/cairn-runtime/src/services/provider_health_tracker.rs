//! In-memory per-provider health statistics for RFC 009 routing.
//!
//! Tracks success rate, latency percentiles (p50/p95), consecutive failures,
//! and last error for each provider connection. Used by [`ProviderRouter`] to
//! make health-aware routing decisions without hitting the event store on
//! every request.

use std::collections::HashMap;
use std::sync::RwLock;

use cairn_domain::ProviderConnectionId;

/// Maximum latency samples kept per provider for percentile calculations.
const LATENCY_RING_SIZE: usize = 100;

/// Per-provider health statistics.
#[derive(Clone, Debug)]
pub struct ProviderHealthStats {
    pub success_count: u64,
    pub failure_count: u64,
    /// Ring buffer of recent latency samples (milliseconds).
    latencies_ms: Vec<u64>,
    /// Write cursor into `latencies_ms`.
    latency_cursor: usize,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub last_check_ms: u64,
}

impl ProviderHealthStats {
    fn new() -> Self {
        Self {
            success_count: 0,
            failure_count: 0,
            latencies_ms: Vec::with_capacity(LATENCY_RING_SIZE),
            latency_cursor: 0,
            consecutive_failures: 0,
            last_error: None,
            last_check_ms: 0,
        }
    }

    fn record_latency(&mut self, ms: u64) {
        if self.latencies_ms.len() < LATENCY_RING_SIZE {
            self.latencies_ms.push(ms);
        } else {
            self.latencies_ms[self.latency_cursor] = ms;
        }
        self.latency_cursor = (self.latency_cursor + 1) % LATENCY_RING_SIZE;
    }

    /// Success rate as a fraction in [0.0, 1.0]. Returns 1.0 when no calls recorded.
    pub fn success_rate(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 1.0;
        }
        self.success_count as f64 / total as f64
    }

    /// Latency at the given percentile (0–100). Returns 0 when no samples.
    pub fn latency_percentile(&self, p: u8) -> u64 {
        if self.latencies_ms.is_empty() {
            return 0;
        }
        let mut sorted = self.latencies_ms.clone();
        sorted.sort_unstable();
        let idx = ((p as f64 / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    /// Latency p50.
    pub fn p50(&self) -> u64 {
        self.latency_percentile(50)
    }

    /// Latency p95.
    pub fn p95(&self) -> u64 {
        self.latency_percentile(95)
    }
}

/// Thread-safe in-memory health tracker for provider connections.
///
/// Designed to be shared via `Arc<ProviderHealthTracker>` across the router
/// and health-check background tasks.
pub struct ProviderHealthTracker {
    stats: RwLock<HashMap<ProviderConnectionId, ProviderHealthStats>>,
}

impl ProviderHealthTracker {
    pub fn new() -> Self {
        Self {
            stats: RwLock::new(HashMap::new()),
        }
    }

    /// Record a successful provider call.
    pub fn record_success(&self, connection_id: &ProviderConnectionId, latency_ms: u64) {
        let mut map = self.stats.write().unwrap_or_else(|e| e.into_inner());
        let entry = map
            .entry(connection_id.clone())
            .or_insert_with(ProviderHealthStats::new);
        entry.success_count += 1;
        entry.consecutive_failures = 0;
        entry.record_latency(latency_ms);
        entry.last_check_ms = now_ms();
    }

    /// Record a failed provider call.
    pub fn record_failure(
        &self,
        connection_id: &ProviderConnectionId,
        latency_ms: u64,
        error: String,
    ) {
        let mut map = self.stats.write().unwrap_or_else(|e| e.into_inner());
        let entry = map
            .entry(connection_id.clone())
            .or_insert_with(ProviderHealthStats::new);
        entry.failure_count += 1;
        entry.consecutive_failures += 1;
        entry.record_latency(latency_ms);
        entry.last_error = Some(error);
        entry.last_check_ms = now_ms();
    }

    /// Snapshot current stats for a provider. Returns `None` if never tracked.
    pub fn get(&self, connection_id: &ProviderConnectionId) -> Option<ProviderHealthStats> {
        let map = self.stats.read().unwrap_or_else(|e| e.into_inner());
        map.get(connection_id).cloned()
    }

    /// Check whether a provider is considered healthy.
    ///
    /// A provider is unhealthy when it has 3+ consecutive failures OR
    /// its success rate drops below 50% (with at least 10 calls recorded).
    pub fn is_healthy(&self, connection_id: &ProviderConnectionId) -> bool {
        let map = self.stats.read().unwrap_or_else(|e| e.into_inner());
        match map.get(connection_id) {
            None => true, // unknown = assume healthy
            Some(stats) => {
                if stats.consecutive_failures >= 3 {
                    return false;
                }
                let total = stats.success_count + stats.failure_count;
                if total >= 10 && stats.success_rate() < 0.5 {
                    return false;
                }
                true
            }
        }
    }

    /// Snapshot all tracked providers.
    pub fn all(&self) -> HashMap<ProviderConnectionId, ProviderHealthStats> {
        self.stats.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Default for ProviderHealthTracker {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(id: &str) -> ProviderConnectionId {
        ProviderConnectionId::new(id)
    }

    #[test]
    fn new_provider_is_healthy() {
        let tracker = ProviderHealthTracker::new();
        assert!(tracker.is_healthy(&conn("unknown")));
        assert!(tracker.get(&conn("unknown")).is_none());
    }

    #[test]
    fn success_rate_tracks_correctly() {
        let tracker = ProviderHealthTracker::new();
        let id = conn("p1");
        tracker.record_success(&id, 100);
        tracker.record_success(&id, 200);
        tracker.record_failure(&id, 50, "timeout".into());

        let stats = tracker.get(&id).unwrap();
        assert_eq!(stats.success_count, 2);
        assert_eq!(stats.failure_count, 1);
        let rate = stats.success_rate();
        assert!((rate - 0.6667).abs() < 0.01);
    }

    #[test]
    fn consecutive_failures_reset_on_success() {
        let tracker = ProviderHealthTracker::new();
        let id = conn("p1");
        tracker.record_failure(&id, 10, "err".into());
        tracker.record_failure(&id, 10, "err".into());
        assert_eq!(tracker.get(&id).unwrap().consecutive_failures, 2);

        tracker.record_success(&id, 50);
        assert_eq!(tracker.get(&id).unwrap().consecutive_failures, 0);
    }

    #[test]
    fn three_consecutive_failures_marks_unhealthy() {
        let tracker = ProviderHealthTracker::new();
        let id = conn("p1");
        for _ in 0..3 {
            tracker.record_failure(&id, 10, "err".into());
        }
        assert!(!tracker.is_healthy(&id));
    }

    #[test]
    fn latency_percentiles() {
        let tracker = ProviderHealthTracker::new();
        let id = conn("p1");
        // Record 1..=100 (100 samples, ring buffer at capacity)
        for ms in 1..=100u64 {
            tracker.record_success(&id, ms);
        }
        let stats = tracker.get(&id).unwrap();
        // With 100 values 1..=100:
        //   p50 index = round(0.50 * 99) = 50 → value 51 (0-indexed sorted)
        //   p95 index = round(0.95 * 99) = 94 → value 95
        assert_eq!(stats.p50(), 51);
        assert_eq!(stats.p95(), 95);
    }

    #[test]
    fn last_error_stored() {
        let tracker = ProviderHealthTracker::new();
        let id = conn("p1");
        tracker.record_failure(&id, 10, "rate limited".into());
        assert_eq!(
            tracker.get(&id).unwrap().last_error.as_deref(),
            Some("rate limited")
        );
    }
}
