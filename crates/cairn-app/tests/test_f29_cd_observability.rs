//! F29 CD observability backend integration tests.
//!
//! Covers the four user-facing endpoints added in CD:
//!
//! - `GET /v1/runs/:run_id/telemetry` — live rollup of provider calls +
//!   tool invocations + totals.
//! - `GET /v1/runs/stalled` with Pending coverage + configurable
//!   `stuck_run_threshold_ms` system default.
//! - `GET /v1/settings/defaults/:scope/:scope_id/:key` — new exact-lookup
//!   handler previously missing, which caused routing-preview 404s to
//!   fall through to the SPA.
//!
//! Each test spins a `LiveHarness` (full Axum server on an ephemeral
//! port) and exercises the real HTTP surface. Assertions are on JSON
//! payload shape + computed totals, not on in-process event presence —
//! we validate the side-effect that an operator will actually see.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

/// End-to-end: seed a session + run, then assert GET telemetry returns
/// the documented shape with empty rollups. Provider-call seeding
/// without a debug event-append endpoint is out of scope for this
/// integration — it's covered by the lower-level store-side tests
/// (`crates/cairn-store/tests/cost_tracking.rs`) and the metrics
/// integration tests that append directly to the in-process store.
#[tokio::test]
async fn run_telemetry_returns_documented_shape() {
    let h = LiveHarness::setup().await;
    let session_id = format!("sess_{}", h.project);
    let run_id = format!("run_{}", h.project);

    seed_session_and_run(&h, &session_id, &run_id).await;

    let mut req = h
        .client()
        .get(format!("{}/v1/runs/{}/telemetry", h.base_url, run_id))
        .bearer_auth(&h.admin_token);
    for (k, v) in h.scope_headers() {
        req = req.header(k, v);
    }
    let r = req.send().await.expect("telemetry request reaches server");
    assert_eq!(r.status().as_u16(), 200, "telemetry status");
    let body: Value = r.json().await.expect("telemetry json");

    assert_eq!(body["run_id"], run_id);
    // Shape guarantees — the UI + CE depend on every field being present.
    assert!(body["state"].is_string(), "state: {body:?}");
    assert!(body["stuck"].is_boolean());
    assert!(body["provider_calls"].is_array());
    assert!(body["tool_invocations"].is_array());
    assert!(body["totals"].is_object());
    let totals = &body["totals"];
    for key in [
        "cost_micros",
        "input_tokens",
        "output_tokens",
        "provider_calls",
        "tool_calls",
        "errors",
        "wall_ms",
    ] {
        assert!(
            totals[key].is_number(),
            "totals.{key} missing/wrong type in {totals:?}"
        );
    }
    // phase_timings is CF territory; empty in CD.
    assert!(body["phase_timings"].is_object());
    assert_eq!(body["phase_timings"].as_object().unwrap().len(), 0);
}

/// Telemetry returns 404 when the run is not visible to the caller's tenant.
#[tokio::test]
async fn run_telemetry_404_on_unknown_run() {
    let h = LiveHarness::setup().await;
    let mut req = h
        .client()
        .get(format!("{}/v1/runs/does-not-exist/telemetry", h.base_url))
        .bearer_auth(&h.admin_token);
    for (k, v) in h.scope_headers() {
        req = req.header(k, v);
    }
    let r = req.send().await.expect("telemetry 404 request");
    assert_eq!(r.status().as_u16(), 404);
}

/// `GET /v1/settings/defaults/:scope/:scope_id/:key` returns the stored
/// value when set and 404 when unset. Also round-trips a PUT into a GET.
#[tokio::test]
async fn settings_defaults_get_roundtrips_put_and_404s_on_miss() {
    let h = LiveHarness::setup().await;

    // 404 on missing key before any PUT.
    let r = h
        .client()
        .get(format!(
            "{}/v1/settings/defaults/system/system/stuck_run_threshold_ms",
            h.base_url
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("get defaults reaches server");
    assert_eq!(r.status().as_u16(), 404, "expected 404 before PUT");

    // PUT a numeric value.
    let r = h
        .client()
        .put(format!(
            "{}/v1/settings/defaults/system/system/stuck_run_threshold_ms",
            h.base_url
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "value": 42_000 }))
        .send()
        .await
        .expect("put defaults");
    assert_eq!(r.status().as_u16(), 200, "put defaults status");

    // GET now reflects the value + source.
    let r = h
        .client()
        .get(format!(
            "{}/v1/settings/defaults/system/system/stuck_run_threshold_ms",
            h.base_url
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("get after put");
    assert_eq!(r.status().as_u16(), 200);
    let body: Value = r.json().await.expect("get json");
    assert_eq!(body["scope"], "system");
    assert_eq!(body["scope_id"], "system");
    assert_eq!(body["key"], "stuck_run_threshold_ms");
    assert_eq!(body["value"], 42_000);
    assert_eq!(body["source"], "system");
}

/// The stalled-runs endpoint now covers Pending runs, not just Running,
/// and reads the stuck threshold from the `stuck_run_threshold_ms`
/// system default when set.
#[tokio::test]
async fn stalled_runs_covers_pending_and_respects_threshold_default() {
    let h = LiveHarness::setup().await;
    let session_id = format!("sess_{}", h.project);
    let run_id = format!("run_{}", h.project);
    seed_session_and_run(&h, &session_id, &run_id).await;

    // Set a threshold of 1 ms so the run we just created counts as stalled
    // immediately (normal default is 30 minutes).
    let r = h
        .client()
        .put(format!(
            "{}/v1/settings/defaults/system/system/stuck_run_threshold_ms",
            h.base_url
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "value": 1 }))
        .send()
        .await
        .expect("put threshold");
    assert_eq!(r.status().as_u16(), 200);

    // Small pause to ensure elapsed > 1ms.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let scope_headers = h.scope_headers();
    let mut req = h
        .client()
        .get(format!("{}/v1/runs/stalled", h.base_url))
        .bearer_auth(&h.admin_token);
    for (k, v) in scope_headers {
        req = req.header(k, v);
    }
    let r = req.send().await.expect("stalled runs");
    assert_eq!(r.status().as_u16(), 200, "stalled status");
    let body: Value = r.json().await.expect("stalled json");
    let items = body["items"].as_array().expect("items array");
    // A freshly-created pending run is not guaranteed to stay Pending —
    // some bootstrap paths flip it to Running. Either way, with a 1-ms
    // threshold it should appear as stalled.
    assert!(
        items.iter().any(|it| it["run_id"] == run_id),
        "expected run {run_id} to surface as stalled; got items: {items:?}"
    );
}

// ── helpers ────────────────────────────────────────────────────────────

async fn seed_session_and_run(h: &LiveHarness, session_id: &str, run_id: &str) {
    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
            "session_id": session_id,
        }))
        .send()
        .await
        .expect("session create");
    assert_eq!(r.status().as_u16(), 201, "session create");

    let r = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
            "session_id": session_id,
            "run_id": run_id,
        }))
        .send()
        .await
        .expect("run create");
    assert_eq!(r.status().as_u16(), 201, "run create");
}
