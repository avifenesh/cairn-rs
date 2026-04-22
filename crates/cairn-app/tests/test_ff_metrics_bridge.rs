//! `/metrics` surfaces FF (ff-observability) metrics alongside cairn's.
//!
//! Regression guard for the PR #117 follow-up: cairn-fabric constructs
//! a shared `ff_observability::Metrics` registry and hands a clone to
//! the FF Engine; cairn-app's `/metrics` handler appends FF's
//! Prometheus text-exposition to the response. FF is compiled with
//! `ff-observability/enabled`, so the real OTEL → Prometheus exporter
//! runs (the no-op shim would return empty text and the assertions
//! below would fail).
//!
//! Why specific metric-name assertions: FF's `real.rs` defines the
//! `# HELP`/`# TYPE` lines even before any samples land, so the names
//! are present on every scrape regardless of whether any request has
//! hit the engine. If FF rename/drops these in a future minor, this
//! test fails and forces us to bump the docs + CHANGELOG in lockstep.
//!
//! We assert three representative names that land in /metrics during
//! normal cairn-app startup:
//!   * `ff_scanner_cycle` — FF's scanners tick in the background from
//!     `Engine::start`, so at least one sample lands quickly after
//!     startup. Counter; OTEL's Prometheus exporter appends `_total`.
//!   * `ff_scanner_cycle_duration` — histogram paired with the above,
//!     recorded in the same code path. OTEL appends `_seconds` for
//!     `unit="s"` instruments → `ff_scanner_cycle_duration_seconds`.
//!   * `ff_cancel_backlog_depth` — observable gauge. Polled by OTEL
//!     on every collection; emitted on first scrape regardless of
//!     whether cairn ever set a non-zero depth.
//!
//! These cover counter + histogram + gauge; enough to catch silent
//! drift if FF renames or removes these metrics in a future version.

mod support;

use support::live_fabric::LiveHarness;

#[tokio::test]
async fn metrics_endpoint_exposes_ff_metrics() {
    let h = LiveHarness::setup().await;

    // FF's fastest scanner (delayed_promoter) ticks at 750 ms; scanner
    // cycles record into `ff_scanner_cycle_total` unconditionally.
    // Wait past one tick so the registry holds at least one sample —
    // otherwise an over-eager scrape races the first cycle and misses
    // the sample lines (HELP/TYPE alone aren't enough for a useful
    // regression guard against silent metric-name drift).
    tokio::time::sleep(std::time::Duration::from_millis(2_000)).await;

    let res = h
        .client()
        .get(format!("{}/metrics", h.base_url))
        .send()
        .await
        .expect("/metrics endpoint reachable");
    assert_eq!(res.status().as_u16(), 200, "/metrics returns 200");
    let ct = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    assert!(
        ct.starts_with("text/plain"),
        "prometheus exposition is text/plain, got {ct:?}"
    );
    let body = res.text().await.expect("body");

    // Cairn's own metrics must still be present — the bridge augments,
    // never replaces.
    assert!(
        body.contains("http_requests_total"),
        "cairn's http_requests_total still present: {body}"
    );

    // FF metrics that must land on /metrics. Names per
    // ff-observability 0.3.2 `real.rs` `mod name`, with OTEL's
    // Prometheus-exporter suffix rules applied (`_total` for counters,
    // `_seconds` for `unit="s"`).
    for expected in [
        "ff_scanner_cycle_total",
        "ff_scanner_cycle_duration_seconds",
        "ff_cancel_backlog_depth",
    ] {
        assert!(
            body.contains(expected),
            "FF metric `{expected}` expected in /metrics body; got:\n{body}"
        );
    }
}
