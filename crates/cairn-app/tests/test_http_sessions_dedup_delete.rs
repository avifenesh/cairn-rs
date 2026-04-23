//! Session create dedup + admin DELETE (closes #229).
//!
//! Before: `POST /v1/sessions` with an already-registered session_id
//! silently returned 201 (idempotent) — UI couldn't distinguish a new
//! session from a collision. Empty + 10k session_id also returned 201.
//! No DELETE route existed, so a typo stayed forever.
//!
//! After: dup → 409, empty/oversized → 422, admin DELETE → 204 +
//! soft-delete via `SessionService::archive` (mirrors PR BB's workspace
//! pattern).

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

fn create_payload(h: &LiveHarness, session_id: &str) -> serde_json::Value {
    json!({
        "tenant_id": h.tenant,
        "workspace_id": h.workspace,
        "project_id": h.project,
        "session_id": session_id,
    })
}

#[tokio::test]
async fn duplicate_session_id_returns_409() {
    let h = LiveHarness::setup().await;
    let session_id = format!("sess-{}", uuid::Uuid::new_v4().simple());

    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&create_payload(&h, &session_id))
        .send()
        .await
        .expect("first create");
    assert_eq!(r.status().as_u16(), 201, "first create should 201");

    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&create_payload(&h, &session_id))
        .send()
        .await
        .expect("duplicate create");
    assert_eq!(
        r.status().as_u16(),
        409,
        "duplicate must 409, not silent 201: body={}",
        r.text().await.unwrap_or_default(),
    );
}

#[tokio::test]
async fn empty_session_id_returns_422() {
    let h = LiveHarness::setup().await;
    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&create_payload(&h, ""))
        .send()
        .await
        .expect("empty-id create");
    assert_eq!(
        r.status().as_u16(),
        422,
        "empty session_id must 422: body={}",
        r.text().await.unwrap_or_default(),
    );
}

#[tokio::test]
async fn oversized_session_id_returns_422() {
    let h = LiveHarness::setup().await;
    let huge = "a".repeat(10_000);
    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&create_payload(&h, &huge))
        .send()
        .await
        .expect("oversized-id create");
    assert_eq!(r.status().as_u16(), 422, "10k session_id must 422");
}

#[tokio::test]
async fn admin_delete_session_returns_204() {
    let h = LiveHarness::setup().await;
    let session_id = format!("sess-{}", uuid::Uuid::new_v4().simple());

    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&create_payload(&h, &session_id))
        .send()
        .await
        .expect("seed create");
    assert_eq!(r.status().as_u16(), 201);

    let r = h
        .client()
        .delete(format!(
            "{}/v1/admin/tenants/{}/sessions/{}",
            h.base_url, h.tenant, session_id
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("admin delete");
    assert_eq!(
        r.status().as_u16(),
        204,
        "admin DELETE should 204: body={}",
        r.text().await.unwrap_or_default(),
    );
}

#[tokio::test]
async fn admin_delete_missing_session_returns_404() {
    let h = LiveHarness::setup().await;
    let r = h
        .client()
        .delete(format!(
            "{}/v1/admin/tenants/{}/sessions/does-not-exist",
            h.base_url, h.tenant
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("admin delete missing");
    assert_eq!(r.status().as_u16(), 404, "missing session DELETE must 404");
}
