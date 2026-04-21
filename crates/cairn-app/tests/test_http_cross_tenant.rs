//! Cross-tenant isolation: identical session/run/task ids in two
//! different tenants must not leak across. Tenant B operator sees 404
//! for tenant A's data — not "forbidden", not "leaked metadata", not
//! "exists but you can't see it". A pure non-existence signal.
//!
//! The test runs against TWO live cairn-app subprocesses driven by the
//! SAME Valkey container. This matches the deployment topology where
//! many operator servers share a single Fabric. Per-tenant isolation
//! is then strictly FF hash-tag + cairn-store scoping — no per-process
//! state leaks in between.
//!
//! Covers task #122: cross-tenant isolation portion of the edge set.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn tenant_b_sees_404_for_tenant_a_task() {
    // Two independent cairn-app subprocesses against the same Valkey.
    let a = LiveHarness::setup().await;
    let b = LiveHarness::setup().await;

    // Identical strings across both tenants — the ONLY thing that
    // differs is the tenant/workspace/project triple.
    let session_id = "shared-session".to_owned();
    let run_id = "shared-run".to_owned();
    let task_id = "shared-task".to_owned();

    // Tenant A: full session -> run -> task chain using its scope.
    seed_chain(&a, &session_id, &run_id, &task_id).await;

    // Tenant B: does NOT seed anything. Now issue reads + mutations
    // that reference the identical ids — but under B's scope + token.
    let list_url = format!(
        "{}/v1/tasks?tenant_id={}&workspace_id={}&project_id={}",
        b.base_url, b.tenant, b.workspace, b.project,
    );
    let res = b
        .client()
        .get(&list_url)
        .bearer_auth(&b.admin_token)
        .send()
        .await
        .expect("list under tenant B reaches server");
    assert_eq!(res.status().as_u16(), 200, "B list endpoint");
    let body: serde_json::Value = res.json().await.expect("list json");
    let items = body
        .as_array()
        .cloned()
        .or_else(|| body.get("items").and_then(|v| v.as_array()).cloned())
        .expect("list body shape");
    assert!(
        items.is_empty(),
        "tenant B saw A's tasks in its list: {}",
        body,
    );

    // Attempt to claim A's task through B's server using identical ids.
    // Must be 404 — not 409, not 403 — to avoid leaking existence.
    let res = b
        .client()
        .post(format!("{}/v1/tasks/{}/claim", b.base_url, task_id))
        .bearer_auth(&b.admin_token)
        .json(&json!({
            "worker_id": format!("worker-b-{}", b.project),
            "lease_duration_ms": 60_000,
        }))
        .send()
        .await
        .expect("claim under tenant B reaches server");
    assert_eq!(
        res.status().as_u16(),
        404,
        "tenant B claiming A's task id leaked a non-404: {}",
        res.text().await.unwrap_or_default(),
    );

    // Attempt to read A's task through B. Must also be 404.
    let res = b
        .client()
        .get(format!("{}/v1/tasks/{}", b.base_url, task_id))
        .bearer_auth(&b.admin_token)
        .send()
        .await
        .expect("read under tenant B reaches server");
    assert_eq!(
        res.status().as_u16(),
        404,
        "tenant B reading A's task id leaked: {}",
        res.text().await.unwrap_or_default(),
    );

    // Finally, tenant A still sees its own task.
    let list_url = format!(
        "{}/v1/tasks?tenant_id={}&workspace_id={}&project_id={}",
        a.base_url, a.tenant, a.workspace, a.project,
    );
    let res = a
        .client()
        .get(&list_url)
        .bearer_auth(&a.admin_token)
        .send()
        .await
        .expect("list under tenant A reaches server");
    let body: serde_json::Value = res.json().await.expect("list json");
    let items = body
        .as_array()
        .cloned()
        .or_else(|| body.get("items").and_then(|v| v.as_array()).cloned())
        .expect("list body shape");
    assert!(
        items
            .iter()
            .any(|t| t.get("task_id").and_then(|v| v.as_str()) == Some(task_id.as_str())),
        "tenant A lost sight of its own task: {}",
        body,
    );
}

async fn seed_chain(h: &LiveHarness, session_id: &str, run_id: &str, task_id: &str) {
    let base = || {
        json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
        })
    };
    let mut body = base();
    body["session_id"] = json!(session_id);
    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&body)
        .send()
        .await
        .expect("session create");
    assert_eq!(r.status().as_u16(), 201);

    let mut body = base();
    body["session_id"] = json!(session_id);
    body["run_id"] = json!(run_id);
    let r = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&body)
        .send()
        .await
        .expect("run create");
    assert_eq!(r.status().as_u16(), 201);

    let mut body = base();
    body["task_id"] = json!(task_id);
    body["parent_run_id"] = json!(run_id);
    let r = h
        .client()
        .post(format!("{}/v1/tasks", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&body)
        .send()
        .await
        .expect("task create");
    assert_eq!(r.status().as_u16(), 201);
}
