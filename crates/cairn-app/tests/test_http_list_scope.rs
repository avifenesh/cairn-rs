//! Issue #234: `GET /v1/tasks` and `GET /v1/runs` must honor the
//! `tenant_id`/`workspace_id`/`project_id` query filters. Previously
//! `list_tasks_filtered` ignored the project key argument and returned
//! every task in the store regardless of scope — a cross-tenant leak
//! even when the admin explicitly asked for a narrower slice.
//!
//! The regression is covered for both the task and run list endpoints
//! by seeding entities in two disjoint projects and asserting that a
//! scoped list returns only the entities in that scope.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

/// Seed a session + run + task triple under the given project triple.
/// Admin can create entities in arbitrary tenants; this is the same
/// pattern used by the normal lifecycle test.
async fn seed(h: &LiveHarness, tenant: &str, workspace: &str, project: &str, suffix: &str) {
    let session_id = format!("sess_{suffix}");
    let run_id = format!("run_{suffix}");
    let task_id = format!("task_{suffix}");

    let res = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "session_id": session_id,
        }))
        .send()
        .await
        .expect("seed session");
    assert_eq!(
        res.status().as_u16(),
        201,
        "seed session {suffix}: {}",
        res.text().await.unwrap_or_default()
    );

    let res = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "session_id": session_id,
            "run_id": run_id,
        }))
        .send()
        .await
        .expect("seed run");
    assert_eq!(
        res.status().as_u16(),
        201,
        "seed run {suffix}: {}",
        res.text().await.unwrap_or_default()
    );

    let res = h
        .client()
        .post(format!("{}/v1/tasks", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "task_id": task_id,
            "parent_run_id": run_id,
        }))
        .send()
        .await
        .expect("seed task");
    assert_eq!(
        res.status().as_u16(),
        201,
        "seed task {suffix}: {}",
        res.text().await.unwrap_or_default()
    );
}

fn items(body: &serde_json::Value) -> Vec<serde_json::Value> {
    body.as_array()
        .cloned()
        .or_else(|| body.get("items").and_then(|v| v.as_array()).cloned())
        .expect("list body is array or {items: [..]}")
}

#[tokio::test]
async fn list_tasks_honors_scope_filter() {
    let h = LiveHarness::setup().await;

    // Project A uses the harness's uuid-scoped triple. Project B shares
    // the same workspace but lives under a distinct tenant so a leak
    // across the tenant axis would show up immediately.
    let tenant_b = format!("{}-b", h.tenant);
    let workspace_b = format!("{}-b", h.workspace);
    let project_b = format!("{}-b", h.project);

    seed(&h, &h.tenant, &h.workspace, &h.project, "a").await;
    seed(&h, &tenant_b, &workspace_b, &project_b, "b").await;

    // Scoped list on project A must not surface anything from B.
    let res = h
        .client()
        .get(format!(
            "{}/v1/tasks?tenant_id={}&workspace_id={}&project_id={}",
            h.base_url, h.tenant, h.workspace, h.project,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list tasks A");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().await.expect("list A json");
    let list = items(&body);
    assert!(
        list.iter()
            .any(|t| t.get("task_id").and_then(|v| v.as_str()) == Some("task_a")),
        "task_a must be in project-A list: {body}"
    );
    assert!(
        list.iter()
            .all(|t| t.get("task_id").and_then(|v| v.as_str()) != Some("task_b")),
        "task_b must NOT leak into project-A list: {body}"
    );

    // And the inverse.
    let res = h
        .client()
        .get(format!(
            "{}/v1/tasks?tenant_id={}&workspace_id={}&project_id={}",
            h.base_url, tenant_b, workspace_b, project_b,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list tasks B");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().await.expect("list B json");
    let list = items(&body);
    assert!(
        list.iter()
            .any(|t| t.get("task_id").and_then(|v| v.as_str()) == Some("task_b")),
        "task_b must be in project-B list: {body}"
    );
    assert!(
        list.iter()
            .all(|t| t.get("task_id").and_then(|v| v.as_str()) != Some("task_a")),
        "task_a must NOT leak into project-B list: {body}"
    );
}

#[tokio::test]
async fn list_runs_honors_scope_filter() {
    let h = LiveHarness::setup().await;

    let tenant_b = format!("{}-b", h.tenant);
    let workspace_b = format!("{}-b", h.workspace);
    let project_b = format!("{}-b", h.project);

    seed(&h, &h.tenant, &h.workspace, &h.project, "a").await;
    seed(&h, &tenant_b, &workspace_b, &project_b, "b").await;

    let res = h
        .client()
        .get(format!(
            "{}/v1/runs?tenant_id={}&workspace_id={}&project_id={}",
            h.base_url, h.tenant, h.workspace, h.project,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list runs A");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().await.expect("list A json");
    let list = items(&body);
    assert!(
        list.iter()
            .any(|r| r.get("run_id").and_then(|v| v.as_str()) == Some("run_a")),
        "run_a must be in project-A list: {body}"
    );
    assert!(
        list.iter()
            .all(|r| r.get("run_id").and_then(|v| v.as_str()) != Some("run_b")),
        "run_b must NOT leak into project-A list: {body}"
    );
}
