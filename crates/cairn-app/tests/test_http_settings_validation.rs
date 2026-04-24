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

#[tokio::test]
async fn accepts_model_id_from_connected_provider_supported_models() {
    // Repro for dogfood findings F20/F21: operator connects OpenRouter with
    // a provider-namespaced ID like `qwen/qwen3-coder:free`. Validator used
    // to reject because the ID doesn't parse as `<known-provider>/<model>`
    // (the split-on-`/` prefix `qwen` isn't a registered provider family).
    // Now the authoritative list is the operator's connection, so this
    // must 200.
    let h = LiveHarness::setup().await;
    let tenant = "default_tenant";
    let suffix = &h.project;
    let connection_id = format!("conn_validator_{suffix}");
    let operator_model = "qwen/qwen3-coder:free";

    // Create credential + connection. The connection exists purely to
    // carry `supported_models` for validation — we don't orchestrate
    // against it in this test.
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-validator-{suffix}"),
        }))
        .send()
        .await
        .expect("credential create");
    assert_eq!(r.status().as_u16(), 201, "credential: {}", r.text().await.unwrap_or_default());
    let credential_id = r
        .json::<serde_json::Value>()
        .await
        .unwrap()
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_owned();

    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "provider_connection_id": connection_id,
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": [operator_model, "minimax/minimax-m2.5:free"],
            "credential_id": credential_id,
            "endpoint_url": "https://openrouter.ai/api/v1",
        }))
        .send()
        .await
        .expect("connection create");
    assert_eq!(r.status().as_u16(), 201, "connection: {}", r.text().await.unwrap_or_default());

    // PUT the operator-namespaced model as the system-wide brain default.
    // Before the fix this returned 422 unknown_model; now must 200.
    let r = put_default(&h, "brain_model", json!(operator_model)).await;
    assert_eq!(
        r.status().as_u16(),
        200,
        "operator-connected model must validate: body={}",
        r.text().await.unwrap_or_default(),
    );

    // Sibling model on the same connection also accepted.
    let r = put_default(&h, "generate_model", json!("minimax/minimax-m2.5:free")).await;
    assert_eq!(
        r.status().as_u16(),
        200,
        "second operator-connected model must validate: body={}",
        r.text().await.unwrap_or_default(),
    );

    // Sanity check the rejection path is still live: a model NOT in the
    // catalog and NOT on any connection still 422s.
    let r = put_default(&h, "brain_model", json!("qwen/not-a-real-route:free")).await;
    assert_eq!(r.status().as_u16(), 422, "uncontested unknown model must still 422");
}

#[tokio::test]
async fn accepts_catalog_id_without_any_connection() {
    // A LiteLLM catalog entry must validate even when no provider
    // connection is configured yet (first-time setup flow: operator picks
    // a model from the catalog before wiring up credentials).
    let h = LiveHarness::setup().await;
    // `gpt-4o` is shipped in the bundled TOML overlay and also in the
    // LiteLLM JSON, so it's always present in `state.model_registry`.
    let r = put_default(&h, "brain_model", json!("gpt-4o")).await;
    assert_eq!(
        r.status().as_u16(),
        200,
        "catalog id must validate sans connection: body={}",
        r.text().await.unwrap_or_default(),
    );
}
