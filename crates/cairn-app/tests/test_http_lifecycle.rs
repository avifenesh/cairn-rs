//! End-to-end HTTP lifecycle: session → run → task → claim → complete.
//!
//! Launches the real cairn-app binary, drives it over HTTP, and
//! verifies each state transition surfaces through the projection layer.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn harness_boots_and_rotates_admin() {
    let harness = LiveHarness::setup().await;
    assert!(harness.base_url.starts_with("http://127.0.0.1:"));
    assert!(!harness.admin_token.is_empty());

    let res = harness
        .client()
        .get(format!("{}/health", harness.base_url))
        .send()
        .await
        .expect("health request reaches server");
    assert!(
        res.status().is_success(),
        "health returned {}",
        res.status()
    );
}

#[tokio::test]
async fn session_run_task_claim_complete_lifecycle() {
    let h = LiveHarness::setup().await;

    let session_id = format!("sess_{}", &h.project);
    let run_id = format!("run_{}", &h.project);
    let task_id = format!("task_{}", &h.project);
    let worker_id = format!("worker_{}", &h.project);

    // 1. Create session.
    let res = h
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
        .expect("POST /v1/sessions reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "session create: {}",
        res.text().await.unwrap_or_default()
    );

    // 2. Create run in that session.
    let res = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
            "session_id": session_id,
            "run_id": run_id,
        }))
        .send()
        .await
        .expect("POST /v1/runs reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "run create: {}",
        res.text().await.unwrap_or_default()
    );

    // 3. Create task parented to that run.
    let res = h
        .client()
        .post(format!("{}/v1/tasks", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
            "task_id": task_id,
            "parent_run_id": run_id,
        }))
        .send()
        .await
        .expect("POST /v1/tasks reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "task create: {}",
        res.text().await.unwrap_or_default()
    );

    // 4. Claim the task. Tenant scope comes from admin bearer, path
    //    carries the task_id.
    let res = h
        .client()
        .post(format!("{}/v1/tasks/{}/claim", h.base_url, task_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "worker_id": worker_id,
            "lease_duration_ms": 60_000,
        }))
        .send()
        .await
        .expect("POST /v1/tasks/{id}/claim reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "task claim: {}",
        res.text().await.unwrap_or_default()
    );
    let claimed: serde_json::Value = res.json().await.expect("claim returns json");
    assert_eq!(
        claimed["state"].as_str(),
        Some("running"),
        "claimed task state = {}",
        claimed,
    );
    assert!(
        claimed["lease_owner"].as_str().is_some(),
        "lease_owner should be set on Leased task: {}",
        claimed,
    );

    // 5. Complete the task.
    let res = h
        .client()
        .post(format!("{}/v1/tasks/{}/complete", h.base_url, task_id))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("POST /v1/tasks/{id}/complete reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "task complete: {}",
        res.text().await.unwrap_or_default()
    );

    // 6. Projection read-back: list tasks in the project and assert the
    //    task is in terminal `completed` state.
    let res = h
        .client()
        .get(format!(
            "{}/v1/tasks?tenant_id={}&workspace_id={}&project_id={}",
            h.base_url, h.tenant, h.workspace, h.project,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/tasks reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "task list: {}",
        res.text().await.unwrap_or_default()
    );
    let body: serde_json::Value = res.json().await.expect("list json");
    // List endpoint returns either `T[]` or `{items: T[]}` — normalize.
    let items = body
        .as_array()
        .cloned()
        .or_else(|| body.get("items").and_then(|v| v.as_array()).cloned())
        .expect("list body is array or {items: [..]}");
    let task = items
        .iter()
        .find(|t| t.get("task_id").and_then(|v| v.as_str()) == Some(task_id.as_str()))
        .unwrap_or_else(|| panic!("task {task_id} not in projection: {body}"));
    assert_eq!(
        task["state"].as_str(),
        Some("completed"),
        "task terminal state: {}",
        task,
    );
}
