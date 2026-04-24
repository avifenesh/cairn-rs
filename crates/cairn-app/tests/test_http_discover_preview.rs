//! Discover-preview e2e: operator probes a provider for its model catalog
//! BEFORE registering a connection record.
//!
//! Part of #251 (dogfood providers UX polish): the Add-Provider wizard
//! now exposes a "Discover" button that POSTs to
//! `/v1/providers/connections/discover-preview` to populate the model
//! list without persisting a connection.

mod support;

use axum::{routing::get, Json, Router};
use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

async fn spawn_openai_compat_mock() -> String {
    let app = Router::new().route(
        "/v1/models",
        get(|| async {
            Json(json!({
                "data": [
                    { "id": "mock/model-a", "context_length": 8192 },
                    { "id": "mock/model-b", "context_length": 32768 },
                    { "id": "mock/embedding-small" },
                ]
            }))
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    format!("http://{addr}/v1")
}

#[tokio::test]
async fn discover_preview_returns_models_without_registering_connection() {
    let h = LiveHarness::setup().await;
    let mock_url = spawn_openai_compat_mock().await;

    // POST discover-preview with the ad-hoc endpoint — no connection should
    // be persisted, but the model list should come back.
    let r = h
        .client()
        .post(format!(
            "{}/v1/providers/connections/discover-preview",
            h.base_url,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "adapter_type": "openai_compat",
            "endpoint_url": mock_url,
            "api_key": "sk-test-preview",
        }))
        .send()
        .await
        .expect("discover-preview reaches server");

    // Read the body once up-front so we can both assert on status and
    // parse JSON from the same buffer — reqwest consumes `r` on either
    // `.text()` or `.json()`, so we can't call both on the same response.
    let status = r.status().as_u16();
    let raw = r.text().await.expect("discover-preview body");
    assert_eq!(status, 200, "discover-preview: {raw}");

    let body: Value = serde_json::from_str(&raw).expect("discover-preview json");
    let models = body
        .get("models")
        .and_then(|v| v.as_array())
        .expect("models array");
    assert!(
        models.len() >= 3,
        "expected at least 3 mock models, got: {body}"
    );

    // Sanity: each model carries at least a model_id.
    for m in models {
        assert!(
            m.get("model_id").and_then(|v| v.as_str()).is_some(),
            "model missing model_id: {m}"
        );
    }

    // Nothing should have been written to the registry — the preview is
    // read-only by contract. A subsequent list call must not surface an
    // "ad-hoc" connection row.
    let r = h
        .client()
        .get(format!(
            "{}/v1/providers/connections?tenant_id=default_tenant",
            h.base_url,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list connections reaches server");
    assert_eq!(r.status().as_u16(), 200);
    let list: Value = r.json().await.expect("list json");
    let items = list
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for item in &items {
        // None of the newly probed entries should carry our mock URL:
        // preview does NOT persist.
        let endpoint = item
            .get("endpoint_url")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            !endpoint.contains(&mock_url),
            "discover-preview leaked a connection row: {item}",
        );
    }
}

#[tokio::test]
async fn discover_preview_rejects_unknown_api_key_ref_with_404() {
    // Addresses Bugbot high-severity finding: unscoped credential lookups
    // must NOT silently succeed. An unknown credential_id must 404, never
    // fall through to an unauthenticated provider probe.
    let h = LiveHarness::setup().await;
    let mock_url = spawn_openai_compat_mock().await;

    let r = h
        .client()
        .post(format!(
            "{}/v1/providers/connections/discover-preview",
            h.base_url,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "adapter_type": "openai_compat",
            "endpoint_url": mock_url,
            "api_key_ref": "cred_nonexistent_xyz",
        }))
        .send()
        .await
        .expect("discover-preview reaches server");

    assert_eq!(
        r.status().as_u16(),
        404,
        "unknown api_key_ref must 404, not leak into unauthenticated probe",
    );
    let body: Value = r.json().await.expect("json");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("credential_not_found"),
    );
}

#[tokio::test]
async fn discover_preview_rejects_dollar_prefixed_inline_api_key() {
    // Defends against the server-env-var exfiltration vector Copilot
    // flagged: a `$FOO` inline api_key combined with a caller-controlled
    // endpoint would otherwise leak `$FOO`'s value via the outbound
    // Authorization header.
    let h = LiveHarness::setup().await;
    let mock_url = spawn_openai_compat_mock().await;

    let r = h
        .client()
        .post(format!(
            "{}/v1/providers/connections/discover-preview",
            h.base_url,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "adapter_type": "openai_compat",
            "endpoint_url": mock_url,
            "api_key": "$CAIRN_ADMIN_TOKEN",
        }))
        .send()
        .await
        .expect("discover-preview reaches server");

    assert_eq!(r.status().as_u16(), 400);
    let body: Value = r.json().await.expect("json");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("inline_api_key_env_expansion_forbidden"),
    );
}

#[tokio::test]
async fn discover_preview_rejects_missing_endpoint_for_openai_compat() {
    let h = LiveHarness::setup().await;

    let r = h
        .client()
        .post(format!(
            "{}/v1/providers/connections/discover-preview",
            h.base_url,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "adapter_type": "openai_compat",
        }))
        .send()
        .await
        .expect("discover-preview reaches server");

    assert_eq!(
        r.status().as_u16(),
        400,
        "missing endpoint_url should 400 for openai_compat",
    );
}
