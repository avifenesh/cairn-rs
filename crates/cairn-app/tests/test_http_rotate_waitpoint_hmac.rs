//! End-to-end tests for `POST /v1/admin/rotate-waitpoint-hmac`.
//!
//! Drives the FF 0.2 rotation FCALL across every partition through
//! the cairn-app HTTP surface. Covers the happy path, idempotent
//! replay, rotation_conflict, input validation, and auth gates.
//!
//! ## Why one `#[tokio::test]` with sub-stages instead of separate tests
//!
//! The waitpoint HMAC hash lives at `ff:sec:{fp:N}:waitpoint_hmac` —
//! partition-scoped, NOT project-scoped. Rotation is inherently a
//! cluster-wide per-partition mutation. Two tests rotating in parallel
//! against the same Valkey stomp on each other's `current_kid`, which
//! breaks replay-detection assertions. Cargo runs test functions in
//! parallel by default, so splitting these into independent
//! `#[tokio::test]` fns would be racy by construction.
//!
//! Running them as ordered stages inside a single test fn is the
//! simplest serialization. Failure of one stage does not mask the
//! others' intent because each stage carries its own
//! `.expect(purpose)` anchor.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

async fn post_rotate(h: &LiveHarness, body: Value) -> reqwest::Response {
    h.client()
        .post(format!("{}/v1/admin/rotate-waitpoint-hmac", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&body)
        .send()
        .await
        .expect("rotate-waitpoint-hmac endpoint reaches server")
}

fn uuid_secret_hex() -> String {
    // 64 hex chars = 32 bytes.
    let s = uuid::Uuid::new_v4().simple().to_string();
    format!("{s}{s}").chars().take(64).collect()
}

#[tokio::test]
async fn rotate_waitpoint_hmac_end_to_end() {
    // Single harness, sequential stages. Each stage uses a kid
    // derived from the harness project uuid to stay isolated from
    // parallel test-binary invocations while still sharing one
    // cairn-app subprocess.
    let h = LiveHarness::setup().await;
    let base_kid = format!("rot-{}", h.project);

    // ── Stage 1: fresh rotation rotates every partition ─────────────
    let kid_a = format!("{base_kid}-a");
    let secret_a = uuid_secret_hex();
    let res = post_rotate(
        &h,
        json!({
            "new_kid": kid_a,
            "new_secret_hex": secret_a,
            "grace_ms": 60_000,
        }),
    )
    .await;
    assert_eq!(
        res.status().as_u16(),
        200,
        "stage 1 rotate: {}",
        res.text().await.unwrap_or_default()
    );
    let body: Value = res.json().await.expect("stage 1 json");
    let rotated = body["rotated"].as_u64().expect("rotated u64");
    assert!(
        rotated > 0,
        "stage 1: rotated > 0 on fresh rotation: {body}"
    );
    assert_eq!(
        body["failed"].as_array().unwrap().len(),
        0,
        "stage 1 no failures: {body}"
    );
    assert_eq!(body["new_kid"].as_str().unwrap(), kid_a);
    let partition_count = rotated + body["noop"].as_u64().unwrap();

    // ── Stage 2: idempotent replay returns noop across every partition ─
    let res = post_rotate(
        &h,
        json!({
            "new_kid": kid_a,
            "new_secret_hex": secret_a,
            "grace_ms": 60_000,
        }),
    )
    .await;
    assert_eq!(res.status().as_u16(), 200, "stage 2 replay");
    let body: Value = res.json().await.expect("stage 2 json");
    // Expect all partitions to noop. Zero fresh rotations.
    assert_eq!(
        body["rotated"].as_u64().unwrap(),
        0,
        "stage 2 no fresh rotations: {body}"
    );
    assert_eq!(
        body["noop"].as_u64().unwrap(),
        partition_count,
        "stage 2 noop count == partition count: {body}",
    );

    // ── Stage 3: same kid + different secret → unanimous rotation_conflict → 400 ─
    let different_secret = uuid_secret_hex();
    let res = post_rotate(
        &h,
        json!({
            "new_kid": kid_a,
            "new_secret_hex": different_secret,
            "grace_ms": 60_000,
        }),
    )
    .await;
    assert_eq!(res.status().as_u16(), 400, "stage 3 conflict → 400");
    let body: Value = res.json().await.expect("stage 3 json");
    assert_eq!(body["code"].as_str().unwrap(), "rotation_conflict");

    // ── Stage 4: fresh kid + new secret still rotates after conflict ─
    let kid_b = format!("{base_kid}-b");
    let secret_b = uuid_secret_hex();
    let res = post_rotate(
        &h,
        json!({
            "new_kid": kid_b,
            "new_secret_hex": secret_b,
            "grace_ms": 60_000,
        }),
    )
    .await;
    assert_eq!(
        res.status().as_u16(),
        200,
        "stage 4 fresh rotation after conflict"
    );
    let body: Value = res.json().await.expect("stage 4 json");
    assert!(
        body["rotated"].as_u64().unwrap() > 0,
        "stage 4 rotated > 0: {body}"
    );

    // ── Stage 5: empty kid → invalid_kid → 400 ──────────────────────
    let res = post_rotate(
        &h,
        json!({
            "new_kid": "",
            "new_secret_hex": uuid_secret_hex(),
            "grace_ms": 1_000,
        }),
    )
    .await;
    assert_eq!(res.status().as_u16(), 400, "stage 5 empty kid → 400");
    let body: Value = res.json().await.expect("stage 5 json");
    assert_eq!(body["code"].as_str().unwrap(), "invalid_kid");

    // ── Stage 6: non-hex secret → invalid_secret_hex → 400 ──────────
    let res = post_rotate(
        &h,
        json!({
            "new_kid": format!("{base_kid}-badhex"),
            "new_secret_hex": "not-hex-xyz",
            "grace_ms": 1_000,
        }),
    )
    .await;
    assert_eq!(res.status().as_u16(), 400, "stage 6 bad hex → 400");
    let body: Value = res.json().await.expect("stage 6 json");
    assert_eq!(body["code"].as_str().unwrap(), "invalid_secret_hex");

    // ── Stage 7: missing bearer → 401 ───────────────────────────────
    let res = h
        .client()
        .post(format!("{}/v1/admin/rotate-waitpoint-hmac", h.base_url))
        .json(&json!({
            "new_kid": format!("{base_kid}-noauth"),
            "new_secret_hex": uuid_secret_hex(),
            "grace_ms": 1_000,
        }))
        .send()
        .await
        .expect("rotate endpoint reaches server");
    assert_eq!(res.status().as_u16(), 401, "stage 7 missing bearer → 401");
}
