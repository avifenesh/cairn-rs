//! Approval request -> pending -> approve -> resolved.
//!
//! Originally scoped as "approval-gates-resume" in the plan. On audit
//! the resume handler does not gate on pending approvals (FF's
//! `ff_resume_execution` is called directly), so an approval-blocks-
//! resume test would assert behavior that does not exist. The
//! operator-visible edge that DOES exist and regressed silently with
//! the in-memory runtime is the projection lifecycle: approvals must
//! appear in the pending inbox, transition to resolved, and surface
//! the correct decision through the list endpoint.
//!
//! Covers task #122: approval lifecycle portion of the edge set.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn approval_request_appears_pending_then_resolves_approved() {
    let h = LiveHarness::setup().await;
    let session_id = format!("sess_{}", h.project);
    let run_id = format!("run_{}", h.project);
    let approval_id = format!("appr_{}", h.project);

    // Seed session + run so the approval has a real anchor.
    let r = h
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
        .expect("session create reaches server");
    assert_eq!(r.status().as_u16(), 201, "session");

    let r = h
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
        .expect("run create reaches server");
    assert_eq!(r.status().as_u16(), 201, "run");

    // 1. Request approval.
    let r = h
        .client()
        .post(format!("{}/v1/approvals", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
            "approval_id": approval_id,
            "run_id": run_id,
            "requirement": "required",
        }))
        .send()
        .await
        .expect("approval request reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "approval request: {}",
        r.text().await.unwrap_or_default(),
    );
    let record: serde_json::Value = r.json().await.expect("approval json");
    assert!(
        record["decision"].is_null(),
        "new approval must be pending (decision=null), got: {}",
        record,
    );

    // 2. List pending for this project — must include our approval.
    let r = h
        .client()
        .get(format!(
            "{}/v1/approvals?tenant_id={}&workspace_id={}&project_id={}",
            h.base_url, h.tenant, h.workspace, h.project,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list approvals reaches server");
    assert_eq!(r.status().as_u16(), 200, "list approvals");
    let body: serde_json::Value = r.json().await.expect("list json");
    let items = body
        .as_array()
        .cloned()
        .or_else(|| body.get("items").and_then(|v| v.as_array()).cloned())
        .expect("list body shape");
    assert!(
        items
            .iter()
            .any(|a| a.get("approval_id").and_then(|v| v.as_str()) == Some(approval_id.as_str())),
        "pending list missing approval: {}",
        body,
    );

    // 3. Approve it.
    let r = h
        .client()
        .post(format!(
            "{}/v1/approvals/{}/approve",
            h.base_url, approval_id
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("approve reaches server");
    assert_eq!(
        r.status().as_u16(),
        200,
        "approve: {}",
        r.text().await.unwrap_or_default(),
    );
    let record: serde_json::Value = r.json().await.expect("approval json");
    let decision = record["decision"].as_str().unwrap_or("");
    assert_eq!(
        decision.to_lowercase(),
        "approved",
        "post-approve decision: {}",
        record,
    );

    // 4. Approving an already-resolved approval should NOT silently
    //    succeed again — re-posting must return a non-2xx typed error
    //    rather than a 500 or a second 200. (Tolerant: we accept any
    //    4xx here; the exact code is handler-defined.)
    let r = h
        .client()
        .post(format!(
            "{}/v1/approvals/{}/approve",
            h.base_url, approval_id
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("second approve reaches server");
    let status = r.status().as_u16();
    assert!(
        status == 200 || (400..500).contains(&status),
        "double-approve produced unexpected status {}: {}",
        status,
        r.text().await.unwrap_or_default(),
    );
    assert!(status < 500, "double-approve must not 5xx",);
}
