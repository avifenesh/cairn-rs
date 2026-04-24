//! PR BL-b — integration surface (verify-installation + local_fs).
//!
//! Covers two end-to-end contracts the new Integrations/ProjectRepos UX
//! depends on:
//!
//! 1. `POST /v1/integrations/github/verify-installation` surfaces a 502
//!    `github_api_error` when the pasted credentials don't match a real
//!    GitHub App. The endpoint never mutates server state, so we can
//!    assert behaviour without network access: the synthetic PEM we
//!    generate is valid RSA (passes `jsonwebtoken` parsing) but the
//!    app_id/installation_id aren't registered upstream, so GitHub
//!    rejects the JWT and we bubble that up as 502.
//!
//! 2. `POST /v1/integrations` with the new `local_fs` provider type
//!    registers successfully, lists back with the expected shape, and
//!    accepts a second attach via
//!    `POST /v1/projects/:project/repos { host: "local_fs" }` which
//!    then appears in the merged repo list with `host == "local_fs"`.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

fn project_path(h: &LiveHarness) -> String {
    format!("{}%2F{}%2F{}", h.tenant, h.workspace, h.project)
}

/// Generate a throw-away PKCS#1 RSA key at test-runtime via the
/// `openssl` CLI so we never commit key material (GitGuardian would
/// flag it, rightly). The public half is not registered with GitHub,
/// so any API call using this key will 401 — which is the branch
/// `verify_github_installation_rejects_unregistered_app` exercises.
fn generate_test_pem() -> String {
    let out = std::process::Command::new("openssl")
        .args(["genrsa", "-traditional", "2048"])
        .output()
        .expect("openssl CLI should be available in the test environment");
    assert!(
        out.status.success(),
        "openssl genrsa failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).expect("openssl output is utf-8")
}

#[tokio::test]
async fn verify_github_installation_rejects_unregistered_app() {
    let h = LiveHarness::setup().await;
    let pem = generate_test_pem();

    let res = h
        .client()
        .post(format!(
            "{}/v1/integrations/github/verify-installation",
            h.base_url
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "app_id": 999_999_999u64,
            "private_key": pem,
            "installation_id": 987_654_321u64,
        }))
        .send()
        .await
        .expect("verify-installation reaches server");

    // Without network access the request fails at reqwest layer; with
    // network it fails at GitHub with 401. Either way the handler
    // surfaces 502 `github_api_error`. The 400 case would only fire
    // if we'd sent a malformed PEM, which we don't.
    let status = res.status().as_u16();
    let body_text = res.text().await.unwrap_or_default();
    assert_eq!(
        status, 502,
        "expected 502 github_api_error, got {status} / {body_text}",
    );
    assert!(
        body_text.contains("github_api_error"),
        "expected github_api_error in body, got {body_text}",
    );
}

#[tokio::test]
async fn verify_github_installation_rejects_empty_private_key() {
    let h = LiveHarness::setup().await;

    let res = h
        .client()
        .post(format!(
            "{}/v1/integrations/github/verify-installation",
            h.base_url
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "app_id": 1u64,
            "private_key": "",
            "installation_id": 1u64,
        }))
        .send()
        .await
        .expect("verify-installation reaches server");

    assert_eq!(res.status().as_u16(), 400);
}

#[tokio::test]
async fn verify_github_installation_rejects_garbage_pem() {
    let h = LiveHarness::setup().await;

    let res = h
        .client()
        .post(format!(
            "{}/v1/integrations/github/verify-installation",
            h.base_url
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "app_id": 1u64,
            "private_key": "not a real PEM",
            "installation_id": 1u64,
        }))
        .send()
        .await
        .expect("verify-installation reaches server");

    assert_eq!(res.status().as_u16(), 400);
}

#[tokio::test]
async fn local_fs_integration_registers_and_lists() {
    let h = LiveHarness::setup().await;

    // Create a real directory on disk so the plugin's path check
    // passes. tempfile gives us one that auto-cleans.
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().to_string_lossy().into_owned();

    let res = h
        .client()
        .post(format!("{}/v1/integrations", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "id": "local-fs-test",
            "type": "local_fs",
            "config": {
                "path": path,
                "display_name": "Test LocalFS",
            },
        }))
        .send()
        .await
        .expect("register reaches server");

    assert_eq!(
        res.status().as_u16(),
        200,
        "register status, body: {}",
        res.text().await.unwrap_or_default(),
    );

    // GET back — should list the new integration with configured=true.
    let res = h
        .client()
        .get(format!("{}/v1/integrations/local-fs-test", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("get reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("get json");
    assert_eq!(
        body.get("id").and_then(|v| v.as_str()),
        Some("local-fs-test")
    );
    assert_eq!(
        body.get("display_name").and_then(|v| v.as_str()),
        Some("Test LocalFS"),
    );
    assert_eq!(body.get("configured").and_then(|v| v.as_bool()), Some(true));
}

#[tokio::test]
async fn local_fs_project_repo_attach_and_list() {
    let h = LiveHarness::setup().await;
    let p = project_path(&h);
    let base = &h.base_url;

    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().to_string_lossy().into_owned();

    // Attach the local path as a local_fs repo.
    let res = h
        .client()
        .post(format!("{base}/v1/projects/{p}/repos"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "repo_id": path,
            "host": "local_fs",
        }))
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
    assert_eq!(body.get("host").and_then(|v| v.as_str()), Some("local_fs"));
    assert_eq!(
        body.get("repo_id").and_then(|v| v.as_str()),
        Some(path.as_str())
    );

    // List should now include the local_fs entry alongside any github repos.
    let res = h
        .client()
        .get(format!("{base}/v1/projects/{p}/repos"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list reaches server");
    let body: Value = res.json().await.expect("list json");
    let repos = body
        .get("repos")
        .and_then(|v| v.as_array())
        .cloned()
        .expect("repos array");
    let has_local = repos.iter().any(|r| {
        r.get("host").and_then(|v| v.as_str()) == Some("local_fs")
            && r.get("repo_id").and_then(|v| v.as_str()) == Some(path.as_str())
    });
    assert!(has_local, "expected local_fs entry in list, got {repos:?}");

    // Detach via the dedicated local-paths endpoint.
    let res = h
        .client()
        .delete(format!("{base}/v1/projects/{p}/local-paths"))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "path": path }))
        .send()
        .await
        .expect("delete reaches server");
    assert_eq!(res.status().as_u16(), 204);
}

#[tokio::test]
async fn project_repo_attach_rejects_unknown_host() {
    let h = LiveHarness::setup().await;
    let p = project_path(&h);

    let res = h
        .client()
        .post(format!("{}/v1/projects/{p}/repos", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "repo_id": "avifenesh/cairn-rs",
            "host": "bitbucket",
        }))
        .send()
        .await
        .expect("attach reaches server");
    assert_eq!(res.status().as_u16(), 400);
}

#[tokio::test]
async fn project_repo_attach_returns_501_for_known_unimplemented_hosts() {
    let h = LiveHarness::setup().await;
    let p = project_path(&h);

    for host in ["gitlab", "gitea", "confluence"] {
        let res = h
            .client()
            .post(format!("{}/v1/projects/{p}/repos", h.base_url))
            .bearer_auth(&h.admin_token)
            .json(&json!({
                "repo_id": "someowner/somerepo",
                "host": host,
            }))
            .send()
            .await
            .expect("attach reaches server");
        assert_eq!(
            res.status().as_u16(),
            501,
            "host {host} should be 501, got {}",
            res.status().as_u16(),
        );
    }
}

#[tokio::test]
async fn project_repo_default_host_is_github_backward_compat() {
    let h = LiveHarness::setup().await;
    let p = project_path(&h);
    let base = &h.base_url;

    // Attach without `host` field — should default to github.
    let res = h
        .client()
        .post(format!("{base}/v1/projects/{p}/repos"))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "repo_id": "avifenesh/cairn-rs" }))
        .send()
        .await
        .expect("attach reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("attach json");
    assert_eq!(body.get("host").and_then(|v| v.as_str()), Some("github"));
}
