//! RFC 016 — project repo allowlist HTTP surface.
//!
//! Covers the attach → list → get → detach → list-empty roundtrip over a
//! live cairn-app subprocess. This is the contract the new
//! `ProjectReposPage` UI consumes; a regression here (shape change, path
//! encoding drift) would break the dogfood workflow that lets an operator
//! kick off issue-sync runs against a real repo.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

/// URL-encode the `tenant/workspace/project` triple into one Axum path
/// segment. Mirrors `encodeURIComponent` in `ui/src/lib/api.ts` and
/// `TriggersPage.tsx`. The backend then splits on `/` in
/// `parse_project_scope`. The harness ids are `[a-z0-9_]+` so only `/`
/// itself needs percent-encoding.
fn project_path(h: &LiveHarness) -> String {
    format!("{}%2F{}%2F{}", h.tenant, h.workspace, h.project)
}

#[tokio::test]
async fn project_repos_attach_list_get_detach_roundtrip() {
    let h = LiveHarness::setup().await;
    let p = project_path(&h);
    let base = &h.base_url;

    // 1. Initially empty.
    let res = h
        .client()
        .get(format!("{base}/v1/projects/{p}/repos"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("initial list reaches server");
    assert_eq!(res.status().as_u16(), 200, "initial list status");
    let body: Value = res.json().await.expect("initial list json");
    assert_eq!(
        body.get("repos")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(0),
        "expected empty repos array, got {body}",
    );

    // 2. Attach a repo.
    let res = h
        .client()
        .post(format!("{base}/v1/projects/{p}/repos"))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "repo_id": "avifenesh/cairn-rs" }))
        .send()
        .await
        .expect("attach reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "attach status, body: {}",
        res.text().await.unwrap_or_default(),
    );
    let body: Value = res.json().await.expect("attach json");
    assert_eq!(
        body.get("repo_id").and_then(|v| v.as_str()),
        Some("avifenesh/cairn-rs"),
    );
    assert_eq!(
        body.get("allowlisted").and_then(|v| v.as_bool()),
        Some(true)
    );

    // 3. List now shows it.
    let res = h
        .client()
        .get(format!("{base}/v1/projects/{p}/repos"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list after attach reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("list json");
    let repos = body
        .get("repos")
        .and_then(|v| v.as_array())
        .cloned()
        .expect("list repos array");
    assert_eq!(repos.len(), 1, "expected 1 repo attached, got {body}");
    assert_eq!(
        repos[0].get("repo_id").and_then(|v| v.as_str()),
        Some("avifenesh/cairn-rs"),
    );

    // 4. Detail endpoint.
    let res = h
        .client()
        .get(format!("{base}/v1/projects/{p}/repos/avifenesh/cairn-rs"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("detail reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("detail json");
    assert_eq!(
        body.get("allowlisted").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        body.get("repo_id").and_then(|v| v.as_str()),
        Some("avifenesh/cairn-rs"),
    );

    // 5. Detach.
    let res = h
        .client()
        .delete(format!("{base}/v1/projects/{p}/repos/avifenesh/cairn-rs"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("detach reaches server");
    assert_eq!(
        res.status().as_u16(),
        204,
        "detach status, body: {}",
        res.text().await.unwrap_or_default(),
    );

    // 6. List is empty again.
    let res = h
        .client()
        .get(format!("{base}/v1/projects/{p}/repos"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list after detach reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("post-detach list json");
    assert_eq!(
        body.get("repos")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(0),
        "expected empty after detach, got {body}",
    );

    // 7. Detail after detach: allowlisted flips to false, repo_id still
    //    echoed back. (No separate 404 contract for detail today — the
    //    handler always returns a body; UI just reads `allowlisted`.)
    let res = h
        .client()
        .get(format!("{base}/v1/projects/{p}/repos/avifenesh/cairn-rs"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("detail after detach reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("post-detach detail json");
    assert_eq!(
        body.get("allowlisted").and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[tokio::test]
async fn project_repos_rejects_malformed_repo_id() {
    let h = LiveHarness::setup().await;
    let p = project_path(&h);

    let res = h
        .client()
        .post(format!("{}/v1/projects/{p}/repos", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "repo_id": "not-a-valid-repo" }))
        .send()
        .await
        .expect("bad-attach reaches server");
    assert_eq!(
        res.status().as_u16(),
        400,
        "expected 400 for malformed repo_id, body: {}",
        res.text().await.unwrap_or_default(),
    );
}
