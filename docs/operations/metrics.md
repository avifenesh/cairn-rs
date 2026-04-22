# Metrics (Prometheus)

`cairn-app` exposes two Prometheus-format endpoints:

| Endpoint | Auth | Purpose |
|---|---|---|
| `GET /metrics` | Unauthenticated (auth-exempt) | Primary scrape target. Emits cairn's own metrics **plus** FlowFabric (FF) metrics from `ff-observability`. |
| `GET /v1/metrics/prometheus` | Admin token | Legacy/internal; cairn metrics only. Kept for tooling that already points at it. |

Point your Prometheus scrape config at `/metrics`.

## Cairn metrics

The exhaustive list lives in the source (`crates/cairn-app/src/metrics.rs`).
Representative names:

- `http_requests_total{method,path,status}`
- `http_request_duration_ms_{bucket,sum,count}{method,path,le}`
- `active_runs_total`
- `active_tasks_total`
- Feature-gated (`metrics-core`): `cairn_runs_created_total`, `cairn_tasks_created_total`, `cairn_tool_invocations_total`, projection-lag + tenant-queue-depth gauges.
- Feature-gated (`metrics-providers`): provider-call counters + duration histograms.

## FF (FlowFabric) metrics

Surfaced by `ff-observability 0.3.2` via its OTEL → Prometheus exporter.
`cairn-fabric` builds `ff-observability` with the `enabled` feature, so the
real exporter (not the no-op shim) runs in every cairn deployment.

The same `Arc<ff_observability::Metrics>` is shared between the FF `Engine`
(which records into it on the hot path) and the `/metrics` handler (which
calls `Metrics::render()` each scrape).

| Metric | Type | Labels | Source |
|---|---|---|---|
| `ff_http_requests_total` | counter | `method`, `path`, `status` | FF HTTP surface (currently unused in cairn; FF records 0 unless ff-server is in-process) |
| `ff_http_request_duration_seconds` | histogram | `method`, `path`, `status` | FF HTTP surface |
| `ff_scanner_cycle_total` | counter | `scanner` | Every engine scanner cycle (lease expiry, attempt timeout, dependency reconciler, etc.) |
| `ff_scanner_cycle_duration_seconds` | histogram | `scanner` | Paired with the above — wall time per cycle |
| `ff_cancel_backlog_depth` | gauge | — | Current cancel-reconciler backlog depth |
| `ff_claim_from_grant_duration_seconds` | histogram | `lane` | `claim_from_grant` latency |
| `ff_lease_renewal_total` | counter | `outcome` (`ok`|`err`) | Per lease renewal attempt |
| `ff_worker_at_capacity_total` | counter | — | Count of claim attempts rejected with WorkerAtCapacity |
| `ff_budget_hit_total` | counter | `dimension` | Budget hard-breach count |
| `ff_quota_hit_total` | counter | `reason` (`rate`|`concurrency`) | Quota admission-block count |

Naming note: FF defines OTEL instrument names without the `_total` /
`_seconds` suffixes. OTEL's Prometheus exporter appends `_total` on
counter instruments and `_seconds` on instruments with `unit="s"`
automatically — so the wire-format names above are what you query on.

Cardinality envelope (worst case, typical deployment): ~690 label-sets,
5–10k underlying series once histogram buckets are counted. See
`ff-observability`'s `real.rs` for the breakdown.

## Silent-drift guard

`crates/cairn-app/tests/test_ff_metrics_bridge.rs` spins up a
`LiveHarness` cairn-app subprocess, scrapes `/metrics`, and asserts
`ff_scanner_cycle_total`, `ff_scanner_cycle_duration_seconds`, and
`ff_cancel_backlog_depth` are present. If FF renames/drops any of these
in a future minor, CI fails and forces this doc + the CHANGELOG to move
in lockstep.
