//! Application metrics collection and Prometheus rendering.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
};

pub(crate) const HTTP_DURATION_BUCKETS_MS: [u64; 10] =
    [5, 10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000];

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RequestCountKey {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) status: u16,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RequestDurationKey {
    pub(crate) method: String,
    pub(crate) path: String,
}

#[derive(Clone, Debug)]
pub(crate) struct HistogramSample {
    pub(crate) bucket_counts: [u64; HTTP_DURATION_BUCKETS_MS.len()],
    pub(crate) sum_ms: u64,
    pub(crate) count: u64,
}

impl Default for HistogramSample {
    fn default() -> Self {
        Self {
            bucket_counts: [0; HTTP_DURATION_BUCKETS_MS.len()],
            sum_ms: 0,
            count: 0,
        }
    }
}

#[derive(Default)]
pub struct AppMetrics {
    request_totals: Mutex<HashMap<RequestCountKey, u64>>,
    request_durations: Mutex<HashMap<RequestDurationKey, HistogramSample>>,
    active_runs_total: AtomicU64,
    active_tasks_total: AtomicU64,
    startup_complete: AtomicBool,
}

impl AppMetrics {
    pub(crate) fn mark_started(&self) {
        self.startup_complete.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_started(&self) -> bool {
        self.startup_complete.load(Ordering::Relaxed)
    }

    pub(crate) fn record_request(&self, method: &str, path: &str, status: u16, latency_ms: u64) {
        {
            let mut totals = self
                .request_totals
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let key = RequestCountKey {
                method: method.to_owned(),
                path: path.to_owned(),
                status,
            };
            *totals.entry(key).or_insert(0) += 1;
        }

        let mut durations = self
            .request_durations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let sample = durations
            .entry(RequestDurationKey {
                method: method.to_owned(),
                path: path.to_owned(),
            })
            .or_default();
        sample.count += 1;
        sample.sum_ms = sample.sum_ms.saturating_add(latency_ms);
        for (idx, bucket) in HTTP_DURATION_BUCKETS_MS.iter().enumerate() {
            if latency_ms <= *bucket {
                sample.bucket_counts[idx] += 1;
            }
        }
    }

    pub(crate) fn set_active_counts(&self, runs: usize, tasks: usize) {
        self.active_runs_total.store(runs as u64, Ordering::Relaxed);
        self.active_tasks_total
            .store(tasks as u64, Ordering::Relaxed);
    }

    /// Approximate latency percentile (p50 or p95) from histogram buckets.
    /// Returns `None` when no requests have been recorded.
    pub(crate) fn latency_percentile(&self, p: f64) -> Option<u64> {
        let durations = self
            .request_durations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut total_count: u64 = 0;
        let mut merged = [0u64; HTTP_DURATION_BUCKETS_MS.len()];
        for sample in durations.values() {
            total_count += sample.count;
            for (i, &c) in sample.bucket_counts.iter().enumerate() {
                merged[i] += c;
            }
        }
        if total_count == 0 {
            return None;
        }
        let target = ((p / 100.0) * total_count as f64).ceil() as u64;
        let mut cumulative = 0u64;
        for (i, &c) in merged.iter().enumerate() {
            cumulative += c;
            if cumulative >= target {
                return Some(HTTP_DURATION_BUCKETS_MS[i]);
            }
        }
        Some(*HTTP_DURATION_BUCKETS_MS.last().unwrap())
    }

    /// Fraction of requests with status >= 400 (0.0–1.0).
    pub(crate) fn error_rate(&self) -> f32 {
        let totals = self
            .request_totals
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut total: u64 = 0;
        let mut errors: u64 = 0;
        for (key, &count) in totals.iter() {
            total += count;
            if key.status >= 400 {
                errors += count;
            }
        }
        if total == 0 {
            0.0
        } else {
            errors as f32 / total as f32
        }
    }

    pub(crate) fn render_prometheus(&self) -> String {
        let totals = self
            .request_totals
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let durations = self
            .request_durations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        let mut lines = vec![
            "# HELP http_requests_total Total HTTP responses by method, path, and status."
                .to_owned(),
            "# TYPE http_requests_total counter".to_owned(),
        ];
        for (key, value) in totals {
            lines.push(format!(
                "http_requests_total{{method=\"{}\",path=\"{}\",status=\"{}\"}} {}",
                prometheus_label(&key.method),
                prometheus_label(&key.path),
                key.status,
                value
            ));
        }

        lines.push(
            "# HELP http_request_duration_ms Request duration histogram in milliseconds."
                .to_owned(),
        );
        lines.push("# TYPE http_request_duration_ms histogram".to_owned());
        for (key, value) in durations {
            for (idx, bucket) in HTTP_DURATION_BUCKETS_MS.iter().enumerate() {
                lines.push(format!(
                    "http_request_duration_ms_bucket{{method=\"{}\",path=\"{}\",le=\"{}\"}} {}",
                    prometheus_label(&key.method),
                    prometheus_label(&key.path),
                    bucket,
                    value.bucket_counts[idx]
                ));
            }
            lines.push(format!(
                "http_request_duration_ms_bucket{{method=\"{}\",path=\"{}\",le=\"+Inf\"}} {}",
                prometheus_label(&key.method),
                prometheus_label(&key.path),
                value.count
            ));
            lines.push(format!(
                "http_request_duration_ms_sum{{method=\"{}\",path=\"{}\"}} {}",
                prometheus_label(&key.method),
                prometheus_label(&key.path),
                value.sum_ms
            ));
            lines.push(format!(
                "http_request_duration_ms_count{{method=\"{}\",path=\"{}\"}} {}",
                prometheus_label(&key.method),
                prometheus_label(&key.path),
                value.count
            ));
        }

        lines.push("# HELP active_runs_total Active non-terminal runs.".to_owned());
        lines.push("# TYPE active_runs_total gauge".to_owned());
        lines.push(format!(
            "active_runs_total {}",
            self.active_runs_total.load(Ordering::Relaxed)
        ));
        lines.push("# HELP active_tasks_total Active non-terminal tasks.".to_owned());
        lines.push("# TYPE active_tasks_total gauge".to_owned());
        lines.push(format!(
            "active_tasks_total {}",
            self.active_tasks_total.load(Ordering::Relaxed)
        ));
        lines.join("\n")
    }
}

fn prometheus_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
