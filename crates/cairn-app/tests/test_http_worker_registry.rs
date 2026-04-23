//! End-to-end HTTP coverage for the worker registry operator surface.
//!
//! Exercises `POST /v1/workers/register`, `GET /v1/workers`,
//! `GET /v1/workers/:id`, `POST /v1/workers/:id/suspend`,
//! `POST /v1/workers/:id/reactivate`, and `GET /v1/fleet`.
//!
//! The UI (`WorkersPage`) reads `/v1/workers` + `/v1/fleet` on mount and
//! mutates via `suspend` / `reactivate`. Regressions here break operator
//! visibility of the actual coder-agent fleet.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

/// Percent-encode every byte outside the RFC 3986 "unreserved" set
/// (`A-Z a-z 0-9 - _ . ~`) so a worker id containing reserved or
/// special characters (e.g. `/`, `#`, spaces, unicode) still routes to
/// the intended handler. This is a stricter superset of
/// `encodeURIComponent` — the JS built-in leaves `!`, `*`, `'`, `(`,
/// `)` unescaped, but on the axum path-segment matcher an over-escaped
/// byte round-trips correctly, so tightening here is always safe.
fn encode_path(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for &b in segment.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[test]
fn encode_path_escapes_reserved_bytes_and_preserves_unreserved() {
    // Unreserved stays as-is.
    assert_eq!(encode_path("worker_abc-123.v2~x"), "worker_abc-123.v2~x");
    // Reserved path bytes must be percent-encoded.
    assert_eq!(encode_path("a/b#c d"), "a%2Fb%23c%20d");
    // Multi-byte UTF-8 is encoded byte-by-byte.
    assert_eq!(encode_path("π"), "%CF%80");
}

#[tokio::test]
async fn worker_registry_lifecycle_surfaces_through_list_get_and_fleet() {
    let h = LiveHarness::setup().await;
    let worker_id = format!("worker_{}", &h.project);
    let enc = encode_path(&worker_id);

    // 1. Register the worker.
    let res = h
        .client()
        .post(format!("{}/v1/workers/register", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "worker_id":    &worker_id,
            "display_name": "lifecycle-test-worker",
        }))
        .send()
        .await
        .expect("POST /v1/workers/register reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "register: {}",
        res.text().await.unwrap_or_default(),
    );

    // 2. GET /v1/workers — the list envelope must include the new worker.
    let res = h
        .client()
        .get(format!("{}/v1/workers", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/workers reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("json");
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .expect("list envelope has `items`");
    let found = items
        .iter()
        .find(|w| w["worker_id"].as_str() == Some(worker_id.as_str()))
        .expect("registered worker appears in list");
    assert_eq!(
        found["status"].as_str(),
        Some("active"),
        "freshly registered worker is active: {found}",
    );

    // 3. GET /v1/workers/:id — detail read-back.
    let res = h
        .client()
        .get(format!("{}/v1/workers/{}", h.base_url, enc))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/workers/:id reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let detail: Value = res.json().await.expect("json");
    assert_eq!(detail["worker_id"].as_str(), Some(worker_id.as_str()));
    assert_eq!(detail["status"].as_str(), Some("active"));

    // 4. Suspend.
    let res = h
        .client()
        .post(format!("{}/v1/workers/{}/suspend", h.base_url, enc))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "reason": "integration-test" }))
        .send()
        .await
        .expect("POST /v1/workers/:id/suspend reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "suspend: {}",
        res.text().await.unwrap_or_default(),
    );

    // Suspension must be reflected by the detail endpoint.
    let res = h
        .client()
        .get(format!("{}/v1/workers/{}", h.base_url, enc))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/workers/:id reaches server");
    let detail: Value = res.json().await.expect("json");
    assert_eq!(
        detail["status"].as_str(),
        Some("suspended"),
        "suspend must flip status: {detail}",
    );

    // GET /v1/fleet — aggregate must also see the suspension (0 active).
    let res = h
        .client()
        .get(format!("{}/v1/fleet", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/fleet reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let fleet: Value = res.json().await.expect("json");
    assert!(
        fleet["total"].as_u64().unwrap_or(0) >= 1,
        "fleet total ≥ 1 after registration: {fleet}",
    );
    let fleet_worker = fleet["workers"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|w| w["worker_id"].as_str() == Some(worker_id.as_str()))
        })
        .expect("fleet includes suspended worker");
    assert_eq!(
        fleet_worker["status"].as_str(),
        Some("suspended"),
        "fleet reflects suspension: {fleet_worker}",
    );

    // 5. Reactivate.
    let res = h
        .client()
        .post(format!("{}/v1/workers/{}/reactivate", h.base_url, enc,))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("POST /v1/workers/:id/reactivate reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "reactivate: {}",
        res.text().await.unwrap_or_default(),
    );

    let res = h
        .client()
        .get(format!("{}/v1/workers/{}", h.base_url, enc))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/workers/:id reaches server");
    let detail: Value = res.json().await.expect("json");
    assert_eq!(
        detail["status"].as_str(),
        Some("active"),
        "reactivate must restore active: {detail}",
    );
}
