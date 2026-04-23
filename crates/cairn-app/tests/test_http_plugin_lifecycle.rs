//! RFC 015 — per-project plugin enable/disable HTTP surface.
//!
//! Covers the contract the `PluginsPage` UI consumes for the marketplace
//! tab: `catalog` → `install` → enable-per-project → disable-per-project.
//!
//! A regression here is exactly what motivated PR
//! `fix/plugins-405-cross-tenant`: the UI was calling
//! `POST /v1/projects/:proj/plugins/:id/enable` (wrong path, wrong method
//! for disable) and the backend 405'd because the real routes are:
//!
//!   `POST   /v1/projects/:proj/plugins/:id`    — enable
//!   `DELETE /v1/projects/:proj/plugins/:id`    — disable
//!
//! This test locks the contract down so a URL / method drift on either
//! side surfaces immediately.

mod support;

use serde_json::Value;
use support::live_fabric::LiveHarness;

/// URL-encode the `tenant/workspace/project` triple into one Axum path
/// segment. Mirrors `encodeURIComponent` in `ui/src/lib/api.ts`.
fn project_path(h: &LiveHarness) -> String {
    format!("{}%2F{}%2F{}", h.tenant, h.workspace, h.project)
}

#[tokio::test]
async fn plugin_catalog_install_enable_disable_roundtrip() {
    let h = LiveHarness::setup().await;
    let p = project_path(&h);
    let base = &h.base_url;
    // The bundled catalog (loaded at `AppState` construction via
    // `MarketplaceService::load_bundled_catalog()`) ships the `github`
    // plugin in state `Listed`. See `crates/cairn-plugin-catalog`.
    let plugin_id = "github";

    // 1. Catalog lists the github plugin.
    let res = h
        .client()
        .get(format!("{base}/v1/plugins/catalog"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("catalog list reaches server");
    assert_eq!(res.status().as_u16(), 200, "catalog list status");
    let body: Value = res.json().await.expect("catalog list json");
    let plugins = body
        .get("plugins")
        .and_then(|v| v.as_array())
        .expect("plugins array in catalog");
    let github = plugins
        .iter()
        .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(plugin_id))
        .unwrap_or_else(|| panic!("bundled catalog missing `github`: {body}"));
    assert_eq!(
        github.get("state").and_then(|v| v.as_str()),
        Some("listed"),
        "fresh catalog entry should be listed, got {github}",
    );

    // 2. Install the plugin tenant-wide.
    let res = h
        .client()
        .post(format!("{base}/v1/plugins/{plugin_id}/install"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("install reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "install status, body: {}",
        res.text().await.unwrap_or_default(),
    );

    // 3. Enable per-project — `POST /v1/projects/:proj/plugins/:id`, no
    //    `/enable` suffix. This is the bug the PR fixes on the FE.
    let res = h
        .client()
        .post(format!("{base}/v1/projects/{p}/plugins/{plugin_id}"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("enable reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "enable status, body: {}",
        res.text().await.unwrap_or_default(),
    );

    // 4. Enabling with the wrong URL (the old FE shape) must NOT succeed.
    //    If this ever stops returning 404/405, the FE/BE contract has
    //    drifted again and someone should notice.
    let res = h
        .client()
        .post(format!("{base}/v1/projects/{p}/plugins/{plugin_id}/enable"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("wrong-suffix enable reaches server");
    assert!(
        matches!(res.status().as_u16(), 404 | 405),
        "old `/enable` suffix must not resolve; got {}",
        res.status().as_u16(),
    );

    // 5. Disable per-project — `DELETE /v1/projects/:proj/plugins/:id`,
    //    no `/disable` suffix, method DELETE (not POST).
    let res = h
        .client()
        .delete(format!("{base}/v1/projects/{p}/plugins/{plugin_id}"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("disable reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "disable status, body: {}",
        res.text().await.unwrap_or_default(),
    );

    // 6. Disable via POST (the old broken FE call) must NOT succeed —
    //    Axum should answer 405 because the path exists but only binds
    //    POST (enable) and DELETE (disable).
    let res = h
        .client()
        .post(format!(
            "{base}/v1/projects/{p}/plugins/{plugin_id}/disable"
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("wrong-suffix disable reaches server");
    assert!(
        matches!(res.status().as_u16(), 404 | 405),
        "old `/disable` suffix must not resolve; got {}",
        res.status().as_u16(),
    );
}
