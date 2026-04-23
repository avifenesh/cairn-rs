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

    assert_eq!(
        r.status().as_u16(),
        200,
        "discover-preview: {}",
        r.text().await.unwrap_or_default(),
    );

    let body: Value = r.json().await.expect("discover-preview json");
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
