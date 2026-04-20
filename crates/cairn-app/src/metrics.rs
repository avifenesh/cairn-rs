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

/// Label set for cairn's entity-lifecycle counters. `tenant` + `workspace`
/// are both required — every cairn domain event carries a `ProjectKey` so
/// the label is always populable, and tenant-level breakdowns are the
/// primary operator concern.
#[cfg(feature = "metrics-core")]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct EntityCountKey {
    pub(crate) tenant: String,
    pub(crate) workspace: String,
    /// Terminal outcome label for `_terminal_total` counters
    /// (`completed` / `failed` / `cancelled`). Empty for
    /// `_created_total` counters.
    pub(crate) outcome: String,
    /// Failure class label for failed-outcome rows. Empty string for
    /// non-failure rows.
    pub(crate) failure_class: String,
}

/// Label set for tool-invocation counters. Tool names are bounded
/// (cairn has a finite tool catalogue), so label cardinality stays
/// in O(|tools|).
#[cfg(feature = "metrics-core")]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ToolInvocationKey {
    pub(crate) tool: String,
    /// `ok`, `error`, or `timeout`.
    pub(crate) outcome: String,
}

/// Lease-expiry counter label — tasks and runs are tracked
/// separately so operators can see which surface is losing workers.
#[cfg(feature = "metrics-core")]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LeaseExpiryKey {
    /// `task` or `run`.
    pub(crate) entity: String,
}

#[derive(Default)]
pub struct AppMetrics {
    request_totals: Mutex<HashMap<RequestCountKey, u64>>,
    request_durations: Mutex<HashMap<RequestDurationKey, HistogramSample>>,
    active_runs_total: AtomicU64,
    active_tasks_total: AtomicU64,
    startup_complete: AtomicBool,

    // ── metrics-core ─────────────────────────────────────────────
    #[cfg(feature = "metrics-core")]
    runs_created: Mutex<HashMap<EntityCountKey, u64>>,
    #[cfg(feature = "metrics-core")]
    runs_terminal: Mutex<HashMap<EntityCountKey, u64>>,
    #[cfg(feature = "metrics-core")]
    tasks_created: Mutex<HashMap<EntityCountKey, u64>>,
    #[cfg(feature = "metrics-core")]
    tasks_terminal: Mutex<HashMap<EntityCountKey, u64>>,
    #[cfg(feature = "metrics-core")]
    tool_invocations: Mutex<HashMap<ToolInvocationKey, u64>>,
    #[cfg(feature = "metrics-core")]
    lease_expiries: Mutex<HashMap<LeaseExpiryKey, u64>>,
    #[cfg(feature = "metrics-core")]
    projection_lag_events: AtomicU64,
    /// Per-tenant queue-depth gauges bundled into one mutex so a
    /// single tenant update takes one lock and one string hash, not
    /// three. Reader side (render_prometheus) clones the whole map
    /// once per scrape.
    #[cfg(feature = "metrics-core")]
    tenant_queue_depth: Mutex<HashMap<String, TenantQueueDepth>>,

    // ── metrics-providers ───────────────────────────────────────
    /// Counter of LLM provider calls labelled by provider family +
    /// model + operation + status. Cardinality caveat: model IDs are
    /// operator-supplied, so a misconfigured cairn can emit many
    /// distinct values. Acceptable trade-off because the feature is
    /// opt-in and per-model dashboards are the primary operator ask.
    #[cfg(feature = "metrics-providers")]
    provider_calls: Mutex<HashMap<ProviderCallKey, u64>>,
    #[cfg(feature = "metrics-providers")]
    provider_call_durations: Mutex<HashMap<ProviderCallDurationKey, HistogramSample>>,
    #[cfg(feature = "metrics-providers")]
    provider_tokens: Mutex<HashMap<ProviderTokenKey, u64>>,
}

/// Per-tenant gauge bundle. Held behind a single mutex so updates
/// are atomic per tenant and reader-side iteration doesn't need to
/// cross-reference three maps.
#[cfg(feature = "metrics-core")]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TenantQueueDepth {
    pub(crate) active_runs: u64,
    pub(crate) active_tasks: u64,
    pub(crate) pending_approvals: u64,
}

// ── metrics-providers types ─────────────────────────────────────

/// Histogram buckets tuned for LLM call latency. Spans from a
/// fast-path embedding (~100 ms) to a long generate on a big model
/// with heavy context (~2 minutes). Adding more buckets hurts
/// scrape size without helping dashboards.
#[cfg(feature = "metrics-providers")]
pub(crate) const PROVIDER_DURATION_BUCKETS_MS: [u64; 10] = [
    100, 250, 500, 1_000, 2_500, 5_000, 10_000, 30_000, 60_000, 120_000,
];

// Belt-and-suspenders: `HistogramSample::bucket_counts` is sized to
// `HTTP_DURATION_BUCKETS_MS.len()`. The provider path reuses the same
// type but indexes with `PROVIDER_DURATION_BUCKETS_MS`; if someone
// grows one array without the other, the loop in
// `record_provider_call` panics at runtime. This static assert turns
// that into a compile error.
#[cfg(feature = "metrics-providers")]
const _: () = assert!(
    PROVIDER_DURATION_BUCKETS_MS.len() == HTTP_DURATION_BUCKETS_MS.len(),
    "provider histogram reuses HistogramSample's fixed-size bucket_counts array — \
     keep PROVIDER_DURATION_BUCKETS_MS and HTTP_DURATION_BUCKETS_MS the same length",
);

#[cfg(feature = "metrics-providers")]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ProviderCallKey {
    pub(crate) provider_connection: String,
    pub(crate) model: String,
    pub(crate) operation_kind: String,
    /// `succeeded` / `failed` / `cancelled`.
    pub(crate) status: String,
}

#[cfg(feature = "metrics-providers")]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ProviderCallDurationKey {
    pub(crate) provider_connection: String,
    pub(crate) model: String,
    pub(crate) operation_kind: String,
}

#[cfg(feature = "metrics-providers")]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ProviderTokenKey {
    pub(crate) provider_connection: String,
    pub(crate) model: String,
    /// `input` or `output`.
    pub(crate) kind: String,
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

    // ── metrics-core recorders ───────────────────────────────────

    #[cfg(feature = "metrics-core")]
    pub fn record_run_created(&self, tenant: &str, workspace: &str) {
        let mut map = self.runs_created.lock().unwrap_or_else(|e| e.into_inner());
        *map.entry(EntityCountKey {
            tenant: tenant.to_owned(),
            workspace: workspace.to_owned(),
            outcome: String::new(),
            failure_class: String::new(),
        })
        .or_insert(0) += 1;
    }

    #[cfg(feature = "metrics-core")]
    pub fn record_run_terminal(
        &self,
        tenant: &str,
        workspace: &str,
        outcome: &str,
        failure_class: Option<&str>,
    ) {
        let mut map = self.runs_terminal.lock().unwrap_or_else(|e| e.into_inner());
        *map.entry(EntityCountKey {
            tenant: tenant.to_owned(),
            workspace: workspace.to_owned(),
            outcome: outcome.to_owned(),
            failure_class: failure_class.unwrap_or("").to_owned(),
        })
        .or_insert(0) += 1;
    }

    #[cfg(feature = "metrics-core")]
    pub fn record_task_created(&self, tenant: &str, workspace: &str) {
        let mut map = self.tasks_created.lock().unwrap_or_else(|e| e.into_inner());
        *map.entry(EntityCountKey {
            tenant: tenant.to_owned(),
            workspace: workspace.to_owned(),
            outcome: String::new(),
            failure_class: String::new(),
        })
        .or_insert(0) += 1;
    }

    #[cfg(feature = "metrics-core")]
    pub fn record_task_terminal(
        &self,
        tenant: &str,
        workspace: &str,
        outcome: &str,
        failure_class: Option<&str>,
    ) {
        let mut map = self
            .tasks_terminal
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *map.entry(EntityCountKey {
            tenant: tenant.to_owned(),
            workspace: workspace.to_owned(),
            outcome: outcome.to_owned(),
            failure_class: failure_class.unwrap_or("").to_owned(),
        })
        .or_insert(0) += 1;
    }

    #[cfg(feature = "metrics-core")]
    pub fn record_tool_invocation(&self, tool: &str, outcome: &str) {
        let mut map = self
            .tool_invocations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *map.entry(ToolInvocationKey {
            tool: tool.to_owned(),
            outcome: outcome.to_owned(),
        })
        .or_insert(0) += 1;
    }

    /// Called by [`crate::lease_history_subscriber`] on each `expired`
    /// frame it processes. `entity` is `"task"` or `"run"`.
    #[cfg(feature = "metrics-core")]
    pub fn record_lease_expiry(&self, entity: &str) {
        let mut map = self
            .lease_expiries
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *map.entry(LeaseExpiryKey {
            entity: entity.to_owned(),
        })
        .or_insert(0) += 1;
    }

    /// Set the event_log head position minus the last-projected
    /// position. A persistently-high value means the projection is
    /// behind the event log — read-model queries will return stale
    /// data.
    #[cfg(feature = "metrics-core")]
    pub fn set_projection_lag(&self, lag_events: u64) {
        self.projection_lag_events
            .store(lag_events, Ordering::Relaxed);
    }

    #[cfg(feature = "metrics-core")]
    pub fn set_tenant_queue_depth(
        &self,
        tenant: &str,
        active_runs: u64,
        active_tasks: u64,
        pending_approvals: u64,
    ) {
        self.tenant_queue_depth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                tenant.to_owned(),
                TenantQueueDepth {
                    active_runs,
                    active_tasks,
                    pending_approvals,
                },
            );
    }

    /// Prune tenant-queue-depth entries not present in `keep`. Called
    /// by the scrape-time refresh so tenants that have been deleted
    /// (or paged out past the enumeration window) stop appearing in
    /// Prometheus output as phantom series.
    #[cfg(feature = "metrics-core")]
    pub fn retain_tenant_queue_depth(&self, keep: &[String]) {
        let keep_set: std::collections::HashSet<&str> = keep.iter().map(String::as_str).collect();
        self.tenant_queue_depth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|k, _| keep_set.contains(k.as_str()));
    }

    // ── metrics-providers recorders ─────────────────────────────

    /// Record a completed provider call. `latency_ms=None` skips the
    /// histogram (some provider errors surface before the call lands,
    /// e.g. admission-denial at the router — no latency to report).
    /// `input_tokens` / `output_tokens` are also optional because
    /// failed calls typically don't report usage.
    #[cfg(feature = "metrics-providers")]
    #[allow(clippy::too_many_arguments)]
    pub fn record_provider_call(
        &self,
        provider_connection: &str,
        model: &str,
        operation_kind: &str,
        status: &str,
        latency_ms: Option<u64>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    ) {
        // Counter — build the key before taking the lock so allocations
        // don't happen under contention.
        let call_key = ProviderCallKey {
            provider_connection: provider_connection.to_owned(),
            model: model.to_owned(),
            operation_kind: operation_kind.to_owned(),
            status: status.to_owned(),
        };
        {
            let mut calls = self
                .provider_calls
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *calls.entry(call_key).or_insert(0) += 1;
        }

        // Duration histogram (only if the call actually returned).
        if let Some(latency_ms) = latency_ms {
            let duration_key = ProviderCallDurationKey {
                provider_connection: provider_connection.to_owned(),
                model: model.to_owned(),
                operation_kind: operation_kind.to_owned(),
            };
            let mut durations = self
                .provider_call_durations
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            // HistogramSample::default() gives us the zeroed
            // bucket_counts array; the static_assert above guarantees
            // its fixed length matches PROVIDER_DURATION_BUCKETS_MS.
            let sample = durations.entry(duration_key).or_default();
            sample.count += 1;
            sample.sum_ms = sample.sum_ms.saturating_add(latency_ms);
            for (idx, bucket) in PROVIDER_DURATION_BUCKETS_MS.iter().enumerate() {
                if latency_ms <= *bucket {
                    sample.bucket_counts[idx] += 1;
                }
            }
        }

        // Token counters. Skip the lock entirely when neither side of
        // the token pair is present; build each key outside the lock
        // so cloning happens before contention. When both are present
        // we pay two key allocations — unavoidable given they differ
        // in the `kind` label, which is part of the hash.
        if input_tokens.is_none() && output_tokens.is_none() {
            return;
        }
        let input_key = input_tokens.map(|_| ProviderTokenKey {
            provider_connection: provider_connection.to_owned(),
            model: model.to_owned(),
            kind: "input".to_owned(),
        });
        let output_key = output_tokens.map(|_| ProviderTokenKey {
            provider_connection: provider_connection.to_owned(),
            model: model.to_owned(),
            kind: "output".to_owned(),
        });
        let mut tokens = self
            .provider_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let (Some(key), Some(n)) = (input_key, input_tokens) {
            *tokens.entry(key).or_insert(0) += u64::from(n);
        }
        if let (Some(key), Some(n)) = (output_key, output_tokens) {
            *tokens.entry(key).or_insert(0) += u64::from(n);
        }
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

    pub fn render_prometheus(&self) -> String {
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

        #[cfg(feature = "metrics-core")]
        self.render_core_into(&mut lines);

        #[cfg(feature = "metrics-providers")]
        self.render_providers_into(&mut lines);

        lines.join("\n")
    }

    #[cfg(feature = "metrics-core")]
    fn render_core_into(&self, lines: &mut Vec<String>) {
        // Sort rows by label-tuple before emitting so the Prometheus
        // output is byte-stable across scrapes. HashMap iteration is
        // nondeterministic; stable output keeps log analysis + golden
        // test assertions sane.
        fn render_entity_counter(
            lines: &mut Vec<String>,
            name: &str,
            help: &str,
            data: &HashMap<EntityCountKey, u64>,
            with_outcome: bool,
        ) {
            lines.push(format!("# HELP {name} {help}"));
            lines.push(format!("# TYPE {name} counter"));
            let mut entries: Vec<(&EntityCountKey, &u64)> = data.iter().collect();
            entries.sort_by(|a, b| {
                (
                    &a.0.tenant,
                    &a.0.workspace,
                    &a.0.outcome,
                    &a.0.failure_class,
                )
                    .cmp(&(
                        &b.0.tenant,
                        &b.0.workspace,
                        &b.0.outcome,
                        &b.0.failure_class,
                    ))
            });
            for (key, value) in entries {
                let mut labels = format!(
                    "tenant=\"{}\",workspace=\"{}\"",
                    prometheus_label(&key.tenant),
                    prometheus_label(&key.workspace),
                );
                if with_outcome {
                    labels.push_str(&format!(
                        ",outcome=\"{}\",failure_class=\"{}\"",
                        prometheus_label(&key.outcome),
                        prometheus_label(&key.failure_class),
                    ));
                }
                lines.push(format!("{name}{{{labels}}} {value}"));
            }
        }

        let runs_created = self
            .runs_created
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        render_entity_counter(
            lines,
            "cairn_runs_created_total",
            "Runs created, labelled by tenant + workspace.",
            &runs_created,
            false,
        );

        let runs_terminal = self
            .runs_terminal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        render_entity_counter(
            lines,
            "cairn_runs_terminal_total",
            "Runs reaching a terminal state (completed/failed/cancelled).",
            &runs_terminal,
            true,
        );

        let tasks_created = self
            .tasks_created
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        render_entity_counter(
            lines,
            "cairn_tasks_created_total",
            "Tasks created, labelled by tenant + workspace.",
            &tasks_created,
            false,
        );

        let tasks_terminal = self
            .tasks_terminal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        render_entity_counter(
            lines,
            "cairn_tasks_terminal_total",
            "Tasks reaching a terminal state (completed/failed/cancelled).",
            &tasks_terminal,
            true,
        );

        let tool_invocations = self
            .tool_invocations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        lines.push(
            "# HELP cairn_tool_invocations_total Tool invocations by name and outcome.".to_owned(),
        );
        lines.push("# TYPE cairn_tool_invocations_total counter".to_owned());
        let mut tool_entries: Vec<_> = tool_invocations.iter().collect();
        tool_entries.sort_by(|a, b| (&a.0.tool, &a.0.outcome).cmp(&(&b.0.tool, &b.0.outcome)));
        for (key, value) in tool_entries {
            lines.push(format!(
                "cairn_tool_invocations_total{{tool=\"{}\",outcome=\"{}\"}} {}",
                prometheus_label(&key.tool),
                prometheus_label(&key.outcome),
                value,
            ));
        }

        let lease_expiries = self
            .lease_expiries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        lines.push(
            "# HELP cairn_lease_expiries_total \
                FF-initiated lease expiries detected by the subscriber, by entity (task/run)."
                .to_owned(),
        );
        lines.push("# TYPE cairn_lease_expiries_total counter".to_owned());
        let mut expiry_entries: Vec<_> = lease_expiries.iter().collect();
        expiry_entries.sort_by(|a, b| a.0.entity.cmp(&b.0.entity));
        for (key, value) in expiry_entries {
            lines.push(format!(
                "cairn_lease_expiries_total{{entity=\"{}\"}} {}",
                prometheus_label(&key.entity),
                value,
            ));
        }

        lines.push(
            "# HELP cairn_projection_lag_events \
                event_log head position minus last-projected position. \
                Persistently > 0 means read-model is behind the log."
                .to_owned(),
        );
        lines.push("# TYPE cairn_projection_lag_events gauge".to_owned());
        lines.push(format!(
            "cairn_projection_lag_events {}",
            self.projection_lag_events.load(Ordering::Relaxed),
        ));

        let tenant_queue_depth = self
            .tenant_queue_depth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut tenant_entries: Vec<(String, TenantQueueDepth)> =
            tenant_queue_depth.into_iter().collect();
        tenant_entries.sort_by(|a, b| a.0.cmp(&b.0));

        lines.push(
            "# HELP cairn_active_runs_by_tenant Active non-terminal runs per tenant.".to_owned(),
        );
        lines.push("# TYPE cairn_active_runs_by_tenant gauge".to_owned());
        for (tenant, depth) in &tenant_entries {
            lines.push(format!(
                "cairn_active_runs_by_tenant{{tenant=\"{}\"}} {}",
                prometheus_label(tenant),
                depth.active_runs,
            ));
        }

        lines.push(
            "# HELP cairn_active_tasks_by_tenant Active non-terminal tasks per tenant.".to_owned(),
        );
        lines.push("# TYPE cairn_active_tasks_by_tenant gauge".to_owned());
        for (tenant, depth) in &tenant_entries {
            lines.push(format!(
                "cairn_active_tasks_by_tenant{{tenant=\"{}\"}} {}",
                prometheus_label(tenant),
                depth.active_tasks,
            ));
        }

        lines.push(
            "# HELP cairn_pending_approvals_by_tenant Pending approvals awaiting decision per tenant."
                .to_owned(),
        );
        lines.push("# TYPE cairn_pending_approvals_by_tenant gauge".to_owned());
        for (tenant, depth) in &tenant_entries {
            lines.push(format!(
                "cairn_pending_approvals_by_tenant{{tenant=\"{}\"}} {}",
                prometheus_label(tenant),
                depth.pending_approvals,
            ));
        }
    }

    #[cfg(feature = "metrics-providers")]
    fn render_providers_into(&self, lines: &mut Vec<String>) {
        // Provider calls counter.
        let calls = self
            .provider_calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        lines.push(
            "# HELP cairn_provider_calls_total \
                LLM provider calls, labelled by provider family + model + operation + status."
                .to_owned(),
        );
        lines.push("# TYPE cairn_provider_calls_total counter".to_owned());
        let mut call_entries: Vec<_> = calls.iter().collect();
        call_entries.sort_by(|a, b| {
            (
                &a.0.provider_connection,
                &a.0.model,
                &a.0.operation_kind,
                &a.0.status,
            )
                .cmp(&(
                    &b.0.provider_connection,
                    &b.0.model,
                    &b.0.operation_kind,
                    &b.0.status,
                ))
        });
        for (key, value) in call_entries {
            lines.push(format!(
                "cairn_provider_calls_total{{provider_connection=\"{}\",model=\"{}\",operation_kind=\"{}\",status=\"{}\"}} {}",
                prometheus_label(&key.provider_connection),
                prometheus_label(&key.model),
                prometheus_label(&key.operation_kind),
                prometheus_label(&key.status),
                value,
            ));
        }

        // Provider call duration histogram.
        let durations = self
            .provider_call_durations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        lines.push(
            "# HELP cairn_provider_call_duration_ms \
                LLM provider call wall-clock latency, labelled by provider family + model + operation."
                .to_owned(),
        );
        lines.push("# TYPE cairn_provider_call_duration_ms histogram".to_owned());
        let mut dur_entries: Vec<_> = durations.iter().collect();
        dur_entries.sort_by(|a, b| {
            (&a.0.provider_connection, &a.0.model, &a.0.operation_kind).cmp(&(
                &b.0.provider_connection,
                &b.0.model,
                &b.0.operation_kind,
            ))
        });
        for (key, sample) in dur_entries {
            let labels = format!(
                "provider_connection=\"{}\",model=\"{}\",operation_kind=\"{}\"",
                prometheus_label(&key.provider_connection),
                prometheus_label(&key.model),
                prometheus_label(&key.operation_kind),
            );
            for (idx, bucket) in PROVIDER_DURATION_BUCKETS_MS.iter().enumerate() {
                lines.push(format!(
                    "cairn_provider_call_duration_ms_bucket{{{labels},le=\"{bucket}\"}} {}",
                    sample.bucket_counts[idx]
                ));
            }
            lines.push(format!(
                "cairn_provider_call_duration_ms_bucket{{{labels},le=\"+Inf\"}} {}",
                sample.count
            ));
            lines.push(format!(
                "cairn_provider_call_duration_ms_sum{{{labels}}} {}",
                sample.sum_ms
            ));
            lines.push(format!(
                "cairn_provider_call_duration_ms_count{{{labels}}} {}",
                sample.count
            ));
        }

        // Token counters.
        let tokens = self
            .provider_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        lines.push(
            "# HELP cairn_provider_tokens_total \
                Tokens billed by LLM providers, by family + model + kind (input/output)."
                .to_owned(),
        );
        lines.push("# TYPE cairn_provider_tokens_total counter".to_owned());
        let mut token_entries: Vec<_> = tokens.iter().collect();
        token_entries.sort_by(|a, b| {
            (&a.0.provider_connection, &a.0.model, &a.0.kind).cmp(&(
                &b.0.provider_connection,
                &b.0.model,
                &b.0.kind,
            ))
        });
        for (key, value) in token_entries {
            lines.push(format!(
                "cairn_provider_tokens_total{{provider_connection=\"{}\",model=\"{}\",kind=\"{}\"}} {}",
                prometheus_label(&key.provider_connection),
                prometheus_label(&key.model),
                prometheus_label(&key.kind),
                value,
            ));
        }
    }
}

fn prometheus_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
