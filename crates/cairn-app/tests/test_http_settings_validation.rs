//! Settings validation server-side (closes #228).
//!
//! `PUT /v1/settings/defaults/:scope/:scope_id/:key` used to accept
//! anything — empty string, 10k chars, `nonsense-model-id` — and return
//! 200. Real operator pain: a typo in the UI silently persisted and
//! the next agent invocation blew up in the provider layer with an
//! opaque error. Now the handler rejects unknown model ids, out-of-range
//! numerics, and oversized strings with a 422.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

async fn put_default(h: &LiveHarness, key: &str, value: serde_json::Value) -> reqwest::Response {
    h.client()
        .put(format!(
            "{}/v1/settings/defaults/system/system/{}",
            h.base_url, key
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "value": value }))
        .send()
        .await
        .expect("put default")
}

#[tokio::test]
async fn rejects_empty_model_id() {
    let h = LiveHarness::setup().await;
    let r = put_default(&h, "brain_model", json!("")).await;
    assert_eq!(
        r.status().as_u16(),
        422,
        "empty brain_model must 422: body={}",
        r.text().await.unwrap_or_default(),
    );
}

#[tokio::test]
async fn rejects_unknown_model_id() {
    let h = LiveHarness::setup().await;
    let r = put_default(&h, "brain_model", json!("completely-made-up-model-xyz")).await;
    assert_eq!(r.status().as_u16(), 422, "unknown model must 422");
}

#[tokio::test]
async fn rejects_oversized_model_id() {
    let h = LiveHarness::setup().await;
    let oversized = "a".repeat(10_000);
    let r = put_default(&h, "generate_model", json!(oversized)).await;
    assert_eq!(r.status().as_u16(), 422, "10k-char model id must 422");
}

#[tokio::test]
async fn rejects_non_numeric_max_tokens() {
    let h = LiveHarness::setup().await;
    let r = put_default(&h, "max_tokens", json!("not a number")).await;
    assert_eq!(r.status().as_u16(), 422, "string max_tokens must 422");
}

#[tokio::test]
async fn rejects_out_of_range_temperature() {
    let h = LiveHarness::setup().await;
    let r = put_default(&h, "temperature", json!(99.0)).await;
    assert_eq!(
        r.status().as_u16(),
        422,
        "temperature outside [0, 2] must 422",
    );
}

#[tokio::test]
async fn accepts_in_range_timeout_ms() {
    let h = LiveHarness::setup().await;
    let r = put_default(&h, "timeout_ms", json!(30_000)).await;
    assert_eq!(
        r.status().as_u16(),
        200,
        "30s timeout_ms should persist: body={}",
        r.text().await.unwrap_or_default(),
    );
}

#[tokio::test]
async fn rejects_oversized_prompt_like_value() {
    let h = LiveHarness::setup().await;
    // Non-model, non-numeric, non-length-capped keys get the 4096 cap.
    let oversized = "x".repeat(5_000);
    let r = put_default(&h, "system_prompt_override", json!(oversized)).await;
    assert_eq!(r.status().as_u16(), 422, "5k-char generic value must 422");
}
