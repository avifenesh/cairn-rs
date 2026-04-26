//! F40: DELETE /v1/providers/connections/:id then POST with the same ID
//! must succeed. Previously, the DELETE handler emitted a bogus
//! `ProviderConnectionRegistered` event with empty fields and `Disabled`
//! status instead of a delete event. The projection kept the row and the
//! re-create path failed with `400 provider_connection conflict`.
//!
//! Fix:
//!   - New `ProviderConnectionDeleted` event variant.
//!   - `ProviderConnectionService::delete` emits it.
//!   - In-memory projection hard-removes the row.
//!   - Create-handler conflict path now routes through
//!     `runtime_error_response` so legitimate ID-collisions return 409.
//!
//! Verified against dogfood re-seed 2026-04-26 where operators hit this
//! when rotating provider connection definitions in-place.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn delete_then_recreate_same_id_succeeds() {
    let h = LiveHarness::setup().await;
    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let connection_id = format!("f40conn_{suffix}");

    // 1. Create a credential so the provider connection has something to link.
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-test-f40-{suffix}"),
        }))
        .send()
        .await
        .expect("credential create reaches server");
    assert_eq!(r.status().as_u16(), 201);
    let credential_id = r
        .json::<Value>()
        .await
        .unwrap()
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_owned();

    // 2. First create.
    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "provider_connection_id": connection_id,
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": ["openrouter/test-f40"],
            "credential_id": credential_id,
            "endpoint_url": "http://127.0.0.1:1",
        }))
        .send()
        .await
        .expect("first create reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "first create: {}",
        r.text().await.unwrap_or_default(),
    );

    // 3. Delete.
    let r = h
        .client()
        .delete(format!(
            "{}/v1/providers/connections/{}",
            h.base_url, connection_id,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("delete reaches server");
    assert_eq!(
        r.status().as_u16(),
        200,
        "delete: {}",
        r.text().await.unwrap_or_default(),
    );

    // 4. Re-create with the same ID — THIS is the F40 fix. Before the fix
    //    this returned 400 "provider_connection conflict: <id>".
    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "provider_connection_id": connection_id,
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": ["openrouter/test-f40-v2"],
            "credential_id": credential_id,
            "endpoint_url": "http://127.0.0.1:2",
        }))
        .send()
        .await
        .expect("recreate reaches server");
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    assert_eq!(
        status, 201,
        "recreate after delete must succeed, got {status}: {body}",
    );

    // 5. GET /v1/providers/connections must list the new connection
    //    with the updated models — proves the projection row reflects
    //    the re-creation, not the stale registration.
    let r = h
        .client()
        .get(format!(
            "{}/v1/providers/connections?tenant_id={}",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list reaches server");
    assert_eq!(r.status().as_u16(), 200);
    let body: Value = r.json().await.expect("list json");
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
        .cloned()
        .unwrap_or_default();
    let found = items.iter().find(|v| {
        v.get("provider_connection_id")
            .and_then(|v| v.as_str())
            .map(|s| s == connection_id)
            .unwrap_or(false)
    });
    let found = found.unwrap_or_else(|| panic!("recreated connection missing from list: {body}"));
    let models = found
        .get("supported_models")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        models
            .iter()
            .any(|m| m.as_str() == Some("openrouter/test-f40-v2")),
        "list must reflect the recreated connection's models, got {models:?}",
    );
}

/// Genuine create-collision — same ID twice, with no delete between —
/// must return 409 CONFLICT (was 400 BAD_REQUEST before F40).
#[tokio::test]
async fn create_duplicate_id_returns_409() {
    let h = LiveHarness::setup().await;
    let suffix = h.project.clone();
    let tenant = "default_tenant".to_owned();
    let connection_id = format!("f40dup_{suffix}");

    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": "openrouter",
            "plaintext_value": format!("sk-test-dup-{suffix}"),
        }))
        .send()
        .await
        .expect("credential reaches server");
    assert_eq!(r.status().as_u16(), 201);
    let credential_id = r
        .json::<Value>()
        .await
        .unwrap()
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_owned();

    let make_body = || {
        json!({
            "tenant_id": tenant,
            "provider_connection_id": connection_id,
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": [],
            "credential_id": credential_id,
            "endpoint_url": "http://127.0.0.1:1",
        })
    };

    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&make_body())
        .send()
        .await
        .expect("first create reaches server");
    assert_eq!(r.status().as_u16(), 201);

    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&make_body())
        .send()
        .await
        .expect("second create reaches server");
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    assert_eq!(
        status, 409,
        "duplicate create must return 409 Conflict, got {status}: {body}",
    );
    assert!(
        body.contains("conflict"),
        "409 body should reference the conflict, got: {body}",
    );
}
