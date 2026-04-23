//! Issue #218 — DELETE /v1/admin/tenants/:t/workspaces/:w soft-delete contract.
//!
//! Locks the following invariants:
//!
//!   1. `POST` creates a workspace and `GET` lists it.
//!   2. `DELETE` returns 204 and removes the workspace from the default list.
//!   3. `GET ?include_archived=true` surfaces the soft-deleted workspace with
//!      a populated `archived_at`.
//!   4. Double-delete is idempotent (still 204).
//!   5. `DELETE` on a non-existent workspace returns 404.
//!   6. `DELETE` with the wrong tenant_id returns 404 (no cross-tenant delete).
//!
//! If any of these break, the WorkspacesPage "Delete" action regresses.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn workspace_delete_soft_deletes_and_filters_list() {
    let h = LiveHarness::setup().await;
    let base = &h.base_url;
    let tenant = &h.tenant;

    // Create two workspaces so we can prove the filter is surgical.
    for (ws_id, name) in [("ws_keep_218", "keep"), ("ws_gone_218", "gone")] {
        let res = h
            .client()
            .post(format!(
                "{base}/v1/admin/tenants/{tenant}/workspaces"
            ))
            .bearer_auth(&h.admin_token)
            .json(&json!({ "workspace_id": ws_id, "name": name }))
            .send()
            .await
            .expect("create workspace reaches server");
        assert_eq!(
            res.status().as_u16(),
            201,
            "create {ws_id}: {}",
            res.text().await.unwrap_or_default(),
        );
    }

    // Default list shows both (plus the harness-bootstrap workspace).
    let body: Value = h
        .client()
        .get(format!("{base}/v1/admin/tenants/{tenant}/workspaces"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list workspaces")
        .json()
        .await
        .expect("list json");
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .expect("items[]");
    assert!(
        items
            .iter()
            .any(|w| w.get("workspace_id").and_then(|s| s.as_str()) == Some("ws_keep_218")),
        "list must include active ws_keep_218: {body}",
    );
    assert!(
        items
            .iter()
            .any(|w| w.get("workspace_id").and_then(|s| s.as_str()) == Some("ws_gone_218")),
        "list must include ws_gone_218 before delete: {body}",
    );

    // Soft-delete ws_gone_218. Expect 204 No Content.
    let res = h
        .client()
        .delete(format!(
            "{base}/v1/admin/tenants/{tenant}/workspaces/ws_gone_218"
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("delete workspace reaches server");
    assert_eq!(
        res.status().as_u16(),
        204,
        "DELETE ws_gone_218: {}",
        res.text().await.unwrap_or_default(),
    );

    // Default list excludes the archived workspace.
    let body: Value = h
        .client()
        .get(format!("{base}/v1/admin/tenants/{tenant}/workspaces"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list after delete")
        .json()
        .await
        .expect("list json");
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .expect("items[]");
    assert!(
        items
            .iter()
            .all(|w| w.get("workspace_id").and_then(|s| s.as_str()) != Some("ws_gone_218")),
        "archived workspace must be filtered out by default: {body}",
    );
    assert!(
        items
            .iter()
            .any(|w| w.get("workspace_id").and_then(|s| s.as_str()) == Some("ws_keep_218")),
        "active workspace must still be listed: {body}",
    );

    // include_archived=true surfaces it again with archived_at populated.
    let body: Value = h
        .client()
        .get(format!(
            "{base}/v1/admin/tenants/{tenant}/workspaces?include_archived=true"
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list include_archived")
        .json()
        .await
        .expect("list json");
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .expect("items[]");
    let gone = items
        .iter()
        .find(|w| w.get("workspace_id").and_then(|s| s.as_str()) == Some("ws_gone_218"))
        .expect("archived workspace surfaced via include_archived");
    assert!(
        gone.get("archived_at")
            .and_then(|v| v.as_u64())
            .is_some(),
        "archived_at must be populated: {gone}",
    );

    // Double-delete is idempotent.
    let res = h
        .client()
        .delete(format!(
            "{base}/v1/admin/tenants/{tenant}/workspaces/ws_gone_218"
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("idempotent delete");
    assert_eq!(res.status().as_u16(), 204, "second DELETE must still be 204");

    // Unknown workspace returns 404.
    let res = h
        .client()
        .delete(format!(
            "{base}/v1/admin/tenants/{tenant}/workspaces/ws_does_not_exist"
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("delete unknown");
    assert_eq!(
        res.status().as_u16(),
        404,
        "DELETE unknown workspace must 404",
    );

    // Wrong tenant also returns 404 (no cross-tenant deletes).
    let res = h
        .client()
        .delete(format!(
            "{base}/v1/admin/tenants/not_my_tenant/workspaces/ws_keep_218"
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("cross-tenant delete");
    assert_eq!(
        res.status().as_u16(),
        404,
        "DELETE with wrong tenant_id must 404, not silently delete",
    );
}
