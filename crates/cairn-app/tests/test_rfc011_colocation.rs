//! RFC-011 co-location integration tests.
//!
//! Uses the feature-gated `GET /v1/admin/debug/partition` endpoint to
//! assert per-session runs and their tasks share the same Valkey
//! partition, while A2A bare tasks route through the solo mint path.
//!
//! Gated by the `debug-endpoints` Cargo feature — the whole file
//! compiles to zero tests without it, matching the server-side gate
//! on the endpoint. Run with:
//!
//!   CAIRN_TEST_VALKEY_URL=redis://localhost:6379 \
//!     cargo test -p cairn-app --test test_rfc011_colocation \
//!     --features debug-endpoints

#![cfg(feature = "debug-endpoints")]

mod support;

use std::collections::HashSet;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

async fn get_partition(h: &LiveHarness, kind: &str, id: &str) -> Value {
    let url = format!(
        "{}/v1/admin/debug/partition?kind={}&id={}",
        h.base_url, kind, id
    );
    let res = h
        .client()
        .get(&url)
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("debug partition endpoint reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "debug partition {kind}={id}: {}",
        res.text().await.unwrap_or_default(),
    );
    res.json().await.expect("debug partition response is json")
}

#[tokio::test]
async fn session_bound_run_and_task_share_a_partition() {
    let h = LiveHarness::setup().await;
    let session_id = format!("sess_{}", h.project);
    let run_id = format!("run_{}", h.project);
    let task_id = format!("task_{}", h.project);

    // Seed session -> run -> task with parent_run_id set.
    assert_eq!(
        h.client()
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
            .expect("POST /v1/sessions reaches server")
            .status()
            .as_u16(),
        201,
    );
    assert_eq!(
        h.client()
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
            .expect("POST /v1/runs reaches server")
            .status()
            .as_u16(),
        201,
    );
    assert_eq!(
        h.client()
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
            .expect("POST /v1/tasks reaches server")
            .status()
            .as_u16(),
        201,
    );

    let run_info = get_partition(&h, "run", &run_id).await;
    let task_info = get_partition(&h, "task", &task_id).await;

    assert_eq!(
        run_info["partition_index"], task_info["partition_index"],
        "run and task must share a partition under RFC-011 co-location: run={run_info}, task={task_info}",
    );
    assert_eq!(
        run_info["partition_hash_tag"], task_info["partition_hash_tag"],
        "hash tags must match: run={run_info}, task={task_info}",
    );
    assert_eq!(
        task_info["session_id"].as_str(),
        run_info["session_id"].as_str(),
        "adapter derived the same session via parent_run_id",
    );
    assert_eq!(
        task_info["derivation"].as_str(),
        Some("session_flow"),
        "task must route through session_flow when parent_run_id resolves to a session",
    );
}

#[tokio::test]
async fn a2a_bare_task_routes_to_solo_partition() {
    let h = LiveHarness::setup().await;

    // A2A protocol submission — no session, no parent.
    let res = h
        .client()
        .post(format!("{}/v1/a2a/tasks", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "task": {
                "kind": "message/send",
                "input": {
                    "content_type": "text/plain",
                    "content": "bare a2a task",
                }
            }
        }))
        .send()
        .await
        .expect("POST /v1/a2a/tasks reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "a2a submit: {}",
        res.text().await.unwrap_or_default(),
    );
    let body: Value = res.json().await.expect("a2a response json");
    let task_id = body["task_id"]
        .as_str()
        .expect("a2a response has task_id")
        .to_owned();

    let info = get_partition(&h, "task", &task_id).await;
    assert!(
        info["session_id"].is_null(),
        "bare A2A task must have null session_id: {info}",
    );
    assert_eq!(
        info["derivation"].as_str(),
        Some("solo"),
        "bare A2A task must route through solo mint path: {info}",
    );
    // Sanity: partition index must be valid (default 256 partitions).
    let idx = info["partition_index"].as_u64().expect("index is u64");
    assert!(
        idx < 1024,
        "partition index outside plausible range: {info}"
    );
}

#[tokio::test]
async fn ten_sessions_spread_across_multiple_partitions() {
    let h = LiveHarness::setup().await;

    // Seed ten deterministic session_ids; the UUIDv5(CAIRN_NAMESPACE, <string>)
    // derivation in id_map is uniform across the 128-bit output, so partition
    // CRC16 is uniform mod 256. Probability all ten collide to a single
    // partition: 256 × (1/256)^9 ≈ 5.4×10⁻²⁰. Not flaky.
    let mut partitions: HashSet<u64> = HashSet::new();
    for i in 0..10 {
        let session_id = format!("colocation_session_{}", i);
        let run_id = format!("colocation_run_{}", i);

        assert_eq!(
            h.client()
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
                .expect("session create reaches server")
                .status()
                .as_u16(),
            201,
        );
        assert_eq!(
            h.client()
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
                .expect("run create reaches server")
                .status()
                .as_u16(),
            201,
        );

        let info = get_partition(&h, "run", &run_id).await;
        partitions.insert(info["partition_index"].as_u64().expect("u64"));
    }

    assert!(
        partitions.len() >= 2,
        "10 distinct session FlowIds must span at least 2 partitions \
         (collision probability ≈ 5.4e-20); got {partitions:?}",
    );
}

#[tokio::test]
async fn parent_task_only_submission_co_locates_via_handler_resolution() {
    // Covers the load-bearing handler-side session resolution at
    // create_task_handler:250. The fabric adapter's submit() and the
    // TaskCreated projection writer both walk parent_run_id -> session
    // ONLY; neither follows parent_task_id. Without the handler
    // resolution, a child task with parent_task_id set but parent_run_id
    // null would silently route to the solo partition.
    let h = LiveHarness::setup().await;
    let session_id = format!("sess_{}", h.project);
    let run_id = format!("run_{}", h.project);
    let parent_task_id = format!("parent_task_{}", h.project);
    let child_task_id = format!("child_task_{}", h.project);

    // Session + run + parent task (all session-bound via parent_run_id).
    assert_eq!(
        h.client()
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
            .expect("session reaches server")
            .status()
            .as_u16(),
        201,
    );
    assert_eq!(
        h.client()
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
            .expect("run reaches server")
            .status()
            .as_u16(),
        201,
    );
    assert_eq!(
        h.client()
            .post(format!("{}/v1/tasks", h.base_url))
            .bearer_auth(&h.admin_token)
            .json(&json!({
                "tenant_id": h.tenant,
                "workspace_id": h.workspace,
                "project_id": h.project,
                "task_id": parent_task_id,
                "parent_run_id": run_id,
            }))
            .send()
            .await
            .expect("parent task reaches server")
            .status()
            .as_u16(),
        201,
    );

    // Child task: parent_task_id only, no parent_run_id.
    assert_eq!(
        h.client()
            .post(format!("{}/v1/tasks", h.base_url))
            .bearer_auth(&h.admin_token)
            .json(&json!({
                "tenant_id": h.tenant,
                "workspace_id": h.workspace,
                "project_id": h.project,
                "task_id": child_task_id,
                "parent_task_id": parent_task_id,
            }))
            .send()
            .await
            .expect("child task reaches server")
            .status()
            .as_u16(),
        201,
    );

    let run_info = get_partition(&h, "run", &run_id).await;
    let parent_info = get_partition(&h, "task", &parent_task_id).await;
    let child_info = get_partition(&h, "task", &child_task_id).await;

    assert_eq!(
        run_info["partition_index"], parent_info["partition_index"],
        "parent task must co-locate with its run",
    );
    assert_eq!(
        run_info["partition_index"], child_info["partition_index"],
        "child task (parent_task_id only) must co-locate via handler-side \
         session resolution; if this fails, create_task_handler:250 lost \
         its parent_task_id -> session walk and sub-sub-tasks are \
         silently on the solo partition",
    );
    assert_eq!(
        child_info["derivation"].as_str(),
        Some("session_flow"),
        "child must take the session_flow path: {child_info}",
    );
}
