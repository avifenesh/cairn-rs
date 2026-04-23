//! Regression for issue #243: `cairn_http_*` Prometheus series stayed at 0
//! despite live traffic because the binary Prometheus handler read from an
//! orphaned `AppMetrics` struct that no middleware wrote to.
//!
//! The fix routed the handler through the lib-side `AppMetrics` (populated
//! by the observability middleware on every request). This test exercises
//! the full path: send traffic through the router, scrape
//! `/v1/metrics/prometheus`-equivalent rendering (via the public lib API),
//! and assert the counters advanced.

mod support;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use cairn_api::auth::AuthPrincipal;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_domain::tenancy::TenantKey;
use cairn_domain::OperatorId;
use tower::ServiceExt;

const TOKEN: &str = "metrics-test-token";

#[tokio::test]
async fn http_requests_total_advances_after_traffic() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let before = state.metrics.http_total_requests();

    // Issue several requests hitting a mix of handlers + 404 paths so
    // the middleware records 2xx and 4xx statuses.
    for _ in 0..5 {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/runs")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Drain the body so the response is fully completed (and the
        // middleware's `on_response` hook fires).
        let _ = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    }
    for _ in 0..3 {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/tasks")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let _ = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    }

    let after = state.metrics.http_total_requests();
    assert!(
        after >= before + 8,
        "cairn_http_requests_total must advance on traffic — \
         before={before} after={after}"
    );

    // Latency percentiles should now be populated (non-None) even if
    // individual buckets differ — we just want proof the reservoir
    // received samples.
    let p50 = state.metrics.http_latency_percentile(50.0);
    let avg = state.metrics.http_avg_latency_ms();
    // p50/avg may be 0ms on a very fast in-memory path; just check
    // that requests_by_path has a real entry.
    let by_path = state.metrics.http_requests_by_path();
    assert!(
        by_path.values().sum::<u64>() >= 8,
        "requests_by_path must sum to at least 8 — got {by_path:?} (p50={p50}, avg={avg})"
    );
}

#[tokio::test]
async fn unauthorized_request_lands_in_error_bucket() {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;

    // No bearer token → auth middleware returns 401, which the
    // observability middleware must count as an error.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let _ = to_bytes(res.into_body(), usize::MAX).await.unwrap();

    let errors = state.metrics.http_errors_by_status();
    let error_rate = state.metrics.http_error_rate();
    assert!(
        errors.get(&401).copied().unwrap_or(0) >= 1,
        "401 must be recorded in errors_by_status — got {errors:?} (rate={error_rate})"
    );
    assert!(
        error_rate > 0.0,
        "error_rate must be > 0 after a 401 — got {error_rate}"
    );
}
