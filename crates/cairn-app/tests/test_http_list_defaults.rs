//! Bare list calls (no tenant/workspace/project query params) must not
//! 422. Regression for #165: /v1/runs, /v1/sessions, /v1/tasks required
//! all three scope params, so first-load UI (and any quick curl probe)
//! got 422 Unprocessable Entity.
//!
//! Fix: the scope fields are optional and default to
//! DEFAULT_TENANT_ID/DEFAULT_WORKSPACE_ID/DEFAULT_PROJECT_ID. The list
//! endpoint then returns 200 with whatever is under the default scope
//! (empty on a fresh harness).

mod support;

use support::live_fabric::LiveHarness;

#[tokio::test]
async fn list_runs_without_query_params_returns_200() {
    let h = LiveHarness::setup().await;
    let r = h
        .client()
        .get(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list runs");
    assert_eq!(
        r.status().as_u16(),
        200,
        "bare /v1/runs must not 422: {}",
        r.text().await.unwrap_or_default(),
    );
}

#[tokio::test]
async fn list_sessions_without_query_params_returns_200() {
    let h = LiveHarness::setup().await;
    let r = h
        .client()
        .get(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list sessions");
    assert_eq!(
        r.status().as_u16(),
        200,
        "bare /v1/sessions must not 422: {}",
        r.text().await.unwrap_or_default(),
    );
}

#[tokio::test]
async fn list_tasks_without_query_params_returns_200() {
    let h = LiveHarness::setup().await;
    let r = h
        .client()
        .get(format!("{}/v1/tasks", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list tasks");
    assert_eq!(
        r.status().as_u16(),
        200,
        "bare /v1/tasks must not 422: {}",
        r.text().await.unwrap_or_default(),
    );
}

/// Empty-string query params are treated as absent (UI sometimes sends
/// `?tenant_id=` when scope hasn't hydrated from localStorage). Covered
/// for all three list endpoints so future regressions where only one
/// query struct keeps the empty-string filtering are caught.
async fn assert_empty_string_params_return_200(path: &str) {
    let h = LiveHarness::setup().await;
    let url = format!(
        "{}{}?tenant_id=&workspace_id=&project_id=",
        h.base_url, path
    );
    let r = h
        .client()
        .get(&url)
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .unwrap_or_else(|e| panic!("list {path} empty params: {e}"));
    assert_eq!(
        r.status().as_u16(),
        200,
        "empty-string params on {path} must not 422: {}",
        r.text().await.unwrap_or_default(),
    );
}

#[tokio::test]
async fn list_runs_with_empty_string_params_returns_200() {
    assert_empty_string_params_return_200("/v1/runs").await;
}

#[tokio::test]
async fn list_sessions_with_empty_string_params_returns_200() {
    assert_empty_string_params_return_200("/v1/sessions").await;
}

#[tokio::test]
async fn list_tasks_with_empty_string_params_returns_200() {
    assert_empty_string_params_return_200("/v1/tasks").await;
}
