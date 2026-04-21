//! HTTP integration tests for `POST /v1/tasks/{id}/dependencies`.
//!
//! Launches the real `cairn-app` binary against a live Valkey and
//! exercises the new `dependency_kind` + `data_passing_ref` surface
//! added on top of PR #66's flow-edge wiring. Covers:
//!
//! - round-trip: declare with a ref, GET echoes it.
//! - 422 on oversized ref (257 bytes).
//! - 422 on disallowed charset.
//! - 422 on unknown `dependency_kind` (serde rejects at the JSON
//!   extractor before reaching the handler).
//! - 409 on re-declare with a different ref.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

/// Shared setup: create a session + a run in that session + two
/// sibling tasks parented to the run so they inherit the run's
/// session binding. Tasks only acquire a `session_id` via
/// `parent_run_id → run.session_id` (the POST /v1/tasks body has no
/// `session_id` field), so the dep-handler's same-session check
/// requires this chain.
async fn setup_two_tasks(h: &LiveHarness) -> (String, String, String) {
    let session_id = format!("sess_{}", &h.project);
    let run_id = format!("run_{}", &h.project);
    let task_a = format!("task_a_{}", &h.project);
    let task_b = format!("task_b_{}", &h.project);

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
    assert_eq!(res.status().as_u16(), 201, "session create");

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

    for task_id in [&task_a, &task_b] {
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
            "task {task_id} create: {}",
            res.text().await.unwrap_or_default()
        );
    }

    (session_id, task_a, task_b)
}

#[tokio::test]
async fn declare_dependency_data_passing_ref_roundtrips_over_http() {
    let h = LiveHarness::setup().await;
    let (_session, task_a, task_b) = setup_two_tasks(&h).await;

    let ref_value = "artifact/v1.42";

    // POST /v1/tasks/:b/dependencies → depends_on=:a, ref=ref_value
    let res = h
        .client()
        .post(format!("{}/v1/tasks/{}/dependencies", h.base_url, task_b))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "depends_on_task_id": task_a,
            "data_passing_ref": ref_value,
        }))
        .send()
        .await
        .expect("POST /dependencies reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "declare: {}",
        res.text().await.unwrap_or_default()
    );
    let body: serde_json::Value = res.json().await.expect("declare json");
    assert_eq!(
        body["dependency"]["data_passing_ref"].as_str(),
        Some(ref_value)
    );
    assert_eq!(
        body["dependency"]["dependency_kind"].as_str(),
        Some("success_only")
    );

    // GET /v1/tasks/:b/dependencies → list should include the ref
    let res = h
        .client()
        .get(format!("{}/v1/tasks/{}/dependencies", h.base_url, task_b))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /dependencies reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let items: Vec<serde_json::Value> = res.json().await.expect("list json");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]["dependency"]["data_passing_ref"].as_str(),
        Some(ref_value),
        "GET response: {items:?}"
    );
}

#[tokio::test]
async fn declare_dependency_rejects_oversized_data_passing_ref() {
    let h = LiveHarness::setup().await;
    let (_session, task_a, task_b) = setup_two_tasks(&h).await;

    // 257 bytes — one over the 256 limit.
    let oversize: String = "a".repeat(257);

    let res = h
        .client()
        .post(format!("{}/v1/tasks/{}/dependencies", h.base_url, task_b))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "depends_on_task_id": task_a,
            "data_passing_ref": oversize,
        }))
        .send()
        .await
        .expect("POST /dependencies reaches server");
    assert_eq!(
        res.status().as_u16(),
        422,
        "expected 422, got body: {}",
        res.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn declare_dependency_rejects_disallowed_charset() {
    let h = LiveHarness::setup().await;
    let (_session, task_a, task_b) = setup_two_tasks(&h).await;

    for bad in ["has spaces", "has\nnewline", "weird=sign", "non-ascii-ñ"] {
        let res = h
            .client()
            .post(format!("{}/v1/tasks/{}/dependencies", h.base_url, task_b))
            .bearer_auth(&h.admin_token)
            .json(&json!({
                "depends_on_task_id": task_a,
                "data_passing_ref": bad,
            }))
            .send()
            .await
            .expect("POST /dependencies reaches server");
        assert_eq!(
            res.status().as_u16(),
            422,
            "charset {bad:?}: {}",
            res.text().await.unwrap_or_default()
        );
    }
}

#[tokio::test]
async fn declare_dependency_rejects_unknown_dependency_kind() {
    let h = LiveHarness::setup().await;
    let (_session, task_a, task_b) = setup_two_tasks(&h).await;

    // serde's rename_all="snake_case" on DependencyKind will reject
    // "only_on_failure" at the JSON extractor layer (the axum Json
    // extractor returns a 422 body for deserialisation failures).
    let res = h
        .client()
        .post(format!("{}/v1/tasks/{}/dependencies", h.base_url, task_b))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "depends_on_task_id": task_a,
            "dependency_kind": "only_on_failure",
        }))
        .send()
        .await
        .expect("POST /dependencies reaches server");
    assert_eq!(
        res.status().as_u16(),
        422,
        "unknown kind should 422, got: {}",
        res.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn declare_dependency_conflict_on_different_ref_over_http() {
    let h = LiveHarness::setup().await;
    let (_session, task_a, task_b) = setup_two_tasks(&h).await;

    // First declare stores ref=v1.
    let res = h
        .client()
        .post(format!("{}/v1/tasks/{}/dependencies", h.base_url, task_b))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "depends_on_task_id": task_a,
            "data_passing_ref": "v1",
        }))
        .send()
        .await
        .expect("first declare reaches server");
    assert_eq!(res.status().as_u16(), 201);

    // Second declare with ref=v2 — 409 dependency_conflict.
    let res = h
        .client()
        .post(format!("{}/v1/tasks/{}/dependencies", h.base_url, task_b))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "depends_on_task_id": task_a,
            "data_passing_ref": "v2",
        }))
        .send()
        .await
        .expect("second declare reaches server");
    assert_eq!(
        res.status().as_u16(),
        409,
        "conflict response: {}",
        res.text().await.unwrap_or_default()
    );
    let body: serde_json::Value = res.json().await.expect("conflict body json");
    assert_eq!(body["code"].as_str(), Some("dependency_conflict"));
    let msg = body["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("v1") && msg.contains("v2"),
        "conflict message should carry both refs: {msg}"
    );

    // Replay with the original ref — idempotent 201.
    let res = h
        .client()
        .post(format!("{}/v1/tasks/{}/dependencies", h.base_url, task_b))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "depends_on_task_id": task_a,
            "data_passing_ref": "v1",
        }))
        .send()
        .await
        .expect("idempotent replay reaches server");
    assert_eq!(res.status().as_u16(), 201);
}
