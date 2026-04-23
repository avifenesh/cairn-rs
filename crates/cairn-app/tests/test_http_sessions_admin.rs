//! Admin-token access to GET /v1/sessions/:id must bypass per-tenant
//! scope checks. Regression for #164: SessionDetailPage showed "No
//! traces" because `get_session_handler` filtered by
//! `tenant_scope.tenant_id()` without honouring `TenantScope.is_admin`.
//!
//! The harness's admin token is the real admin service account. The
//! session is seeded under a uuid-scoped tenant triple that does NOT
//! match whatever `tenant_id` the admin token's `TenantScope` resolves
//! to (admin uses the default tenant binding). Before the fix this
//! returned 404; after the fix it returns 200 with the seeded session.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn admin_token_reads_session_under_nondefault_tenant() {
    let h = LiveHarness::setup().await;
    let session_id = format!("sess-{}", uuid::Uuid::new_v4().simple());

    // Seed a session under h.tenant (a uuid-scoped non-default tenant).
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
    assert_eq!(
        r.status().as_u16(),
        201,
        "seed failed: {:?}",
        r.text().await
    );

    // Fetch with admin token but NO scope query params on the detail
    // endpoint (the path doesn't take them). Must succeed because admin
    // bypasses the scope filter.
    let r = h
        .client()
        .get(format!("{}/v1/sessions/{}", h.base_url, session_id))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("get session");
    assert_eq!(
        r.status().as_u16(),
        200,
        "admin GET /v1/sessions/:id must not 404 on non-default tenant: {}",
        r.text().await.unwrap_or_default(),
    );
    let body: serde_json::Value = r.json().await.expect("json body");
    assert_eq!(
        body["session"]["session_id"].as_str(),
        Some(session_id.as_str()),
        "unexpected body: {body}",
    );
}
