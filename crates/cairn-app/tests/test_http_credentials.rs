//! HTTP contract tests for `POST /v1/admin/tenants/:t/credentials`.
//!
//! Regression for issue #217: before the fix, posting two credentials
//! with the same `(tenant_id, provider_id)` both returned 201 and
//! silently accumulated in the tenant's credential list. Operators had
//! no way to detect the duplicate without diffing `list` responses,
//! and downstream provider-registry lookups would non-deterministically
//! bind to one of them.
//!
//! The contract after the fix:
//!   * First POST → 201 Created.
//!   * Second POST with the same `provider_id` → 409 Conflict with code
//!     `credential_exists`.
//!   * `GET …/credentials` returns exactly one record for that provider.
//!   * Revoking the first credential unblocks the rotation path: a
//!     subsequent POST for the same `provider_id` succeeds.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn duplicate_credential_for_same_provider_returns_409_and_no_silent_accumulation() {
    let h = LiveHarness::setup().await;

    // Admin token authenticates as the bootstrap tenant. The store is
    // per-subprocess so even fixed `tenant`/`provider_id` values are
    // isolated from other concurrent tests.
    let tenant = "default_tenant";
    let suffix = h.project.clone();
    let provider_id = format!("openrouter-dup-{suffix}");

    // First create — must succeed.
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": provider_id,
            "plaintext_value": format!("sk-first-{suffix}"),
        }))
        .send()
        .await
        .expect("first credential create reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "first credential create must succeed: {}",
        r.text().await.unwrap_or_default(),
    );
    let first_id = r
        .json::<Value>()
        .await
        .expect("first credential json")
        .get("id")
        .and_then(|v| v.as_str())
        .expect("first credential id")
        .to_owned();

    // Second create with the same provider_id — must 409, not 201.
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": provider_id,
            "plaintext_value": format!("sk-second-{suffix}"),
        }))
        .send()
        .await
        .expect("second credential create reaches server");
    let status = r.status().as_u16();
    let body_text = r.text().await.unwrap_or_default();
    assert_eq!(
        status, 409,
        "duplicate provider_id must be 409 Conflict (was 201 before #217): body={body_text}",
    );
    let body: Value = serde_json::from_str(&body_text)
        .unwrap_or_else(|e| panic!("409 body must be JSON: {e}; body={body_text}"));
    assert_eq!(
        body.get("code").and_then(|c| c.as_str()),
        Some("credential_exists"),
        "409 body must carry code=credential_exists: {body:?}",
    );
    let msg = body
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or_default();
    assert!(
        msg.contains(&provider_id) && msg.contains(tenant),
        "409 message must name provider and tenant: {msg:?}",
    );

    // List — must contain exactly one active credential for this provider.
    let r = h
        .client()
        .get(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list credentials reaches server");
    assert_eq!(r.status().as_u16(), 200);
    let list: Value = r.json().await.expect("list json");
    let items = list
        .get("items")
        .and_then(|v| v.as_array())
        .expect("list items");
    let matches: Vec<&Value> = items
        .iter()
        .filter(|c| c.get("provider_id").and_then(|p| p.as_str()) == Some(&provider_id))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "exactly 1 credential must exist for provider_id={provider_id}; got {}: {items:?}",
        matches.len(),
    );
    assert_eq!(
        matches[0].get("id").and_then(|v| v.as_str()),
        Some(first_id.as_str()),
        "the surviving credential must be the first one",
    );

    // Revoke the first, then re-create — must now succeed. This protects
    // the rotate-by-revoke-then-create workflow from the uniqueness check.
    let r = h
        .client()
        .delete(format!(
            "{}/v1/admin/tenants/{}/credentials/{}",
            h.base_url, tenant, first_id,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("revoke reaches server");
    assert_eq!(
        r.status().as_u16(),
        200,
        "revoke: {}",
        r.text().await.unwrap_or_default(),
    );

    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "provider_id": provider_id,
            "plaintext_value": format!("sk-third-{suffix}"),
        }))
        .send()
        .await
        .expect("post-revoke credential create reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "re-creating after revoke must succeed (rotate-by-revoke-then-create): {}",
        r.text().await.unwrap_or_default(),
    );
}
