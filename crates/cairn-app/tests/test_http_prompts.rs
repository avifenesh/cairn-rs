//! HTTP integration — prompt asset + initial version + release authoring.
//!
//! Closes #150: the UI can create an asset and an initial version in one
//! flow without minting IDs client-side; the server mints `pv_<uuid>` and
//! `rel_<uuid>` when the request omits them.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn prompt_asset_version_and_release_with_server_minted_ids() {
    let h = LiveHarness::setup().await;

    let asset_id = format!("asset_{}", &h.project);

    // 1. Create asset.
    let res = h
        .client()
        .post(format!("{}/v1/prompts/assets", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":       h.tenant,
            "workspace_id":    h.workspace,
            "project_id":      h.project,
            "prompt_asset_id": asset_id,
            "name":            "initial-version test",
            "kind":            "system",
        }))
        .send()
        .await
        .expect("create asset");
    assert!(
        res.status().is_success(),
        "create asset returned {}",
        res.status()
    );

    // 2. Create a version WITHOUT supplying prompt_version_id — server
    //    must mint one prefixed with `pv_`.
    let res = h
        .client()
        .post(format!(
            "{}/v1/prompts/assets/{}/versions",
            h.base_url, asset_id
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    h.tenant,
            "workspace_id": h.workspace,
            "project_id":   h.project,
            "content":      "You are a helpful assistant.",
            "content_hash": "sha256:deadbeef",
        }))
        .send()
        .await
        .expect("create version");
    assert!(
        res.status().is_success(),
        "create version returned {}",
        res.status()
    );
    let version_body: serde_json::Value = res.json().await.expect("version json");
    let version_id = version_body["prompt_version_id"]
        .as_str()
        .expect("prompt_version_id present")
        .to_owned();
    assert!(
        version_id.starts_with("pv_"),
        "server should mint pv_<uuid> when omitted; got {version_id}"
    );

    // 3. Asset should now list the version.
    let res = h
        .client()
        .get(format!(
            "{}/v1/prompts/assets/{}/versions",
            h.base_url, asset_id
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list versions");
    assert!(res.status().is_success());
    let listed: serde_json::Value = res.json().await.expect("versions json");
    let items = listed["items"].as_array().expect("items array");
    assert!(
        items
            .iter()
            .any(|v| v["prompt_version_id"] == json!(version_id.clone())),
        "version not present in listing"
    );

    // 4. Create a release WITHOUT prompt_release_id — server mints rel_<uuid>.
    let res = h
        .client()
        .post(format!("{}/v1/prompts/releases", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":         h.tenant,
            "workspace_id":      h.workspace,
            "project_id":        h.project,
            "prompt_asset_id":   asset_id,
            "prompt_version_id": version_id,
        }))
        .send()
        .await
        .expect("create release");
    assert!(
        res.status().is_success(),
        "create release returned {}",
        res.status()
    );
    let release_body: serde_json::Value = res.json().await.expect("release json");
    let release_id = release_body["prompt_release_id"]
        .as_str()
        .expect("prompt_release_id present");
    assert!(
        release_id.starts_with("rel_"),
        "server should mint rel_<uuid> when omitted; got {release_id}"
    );
}
