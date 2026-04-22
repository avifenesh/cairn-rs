//! Meta-integration test for LiveHarness SIGKILL+restart support.
//!
//! Since this IS test infrastructure, its only honest test is end-to-end
//! usage: spawn a real cairn-app subprocess, mutate durable state, kill
//! the subprocess, restart it, and verify the state survives. This is the
//! prerequisite capability for RFC 020 Tracks 1/3/4 — they need to exercise
//! crash-restart cycles against the real binary.

mod support;

use std::time::Duration;

use serde_json::json;
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn harness_sigkill_and_restart_works() {
    // SQLite-backed harness so the event log survives the subprocess.
    let mut h = LiveHarness::setup_with_sqlite().await;

    // 1. Subprocess A is live and serving /health.
    let res = h
        .client()
        .get(format!("{}/health", h.base_url))
        .send()
        .await
        .expect("health reaches subprocess A");
    assert!(
        res.status().is_success(),
        "subprocess A /health: {}",
        res.status()
    );

    // 2. Mutate durable state: create a session. This writes to the event
    //    log (SQLite) and is observable through the sessions projection.
    let session_id = format!("sess_{}", &h.project);
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
        .expect("session create reaches subprocess A");
    assert_eq!(
        res.status().as_u16(),
        201,
        "session create on A: {}",
        res.text().await.unwrap_or_default(),
    );

    // 2b. Sanity check: the session is observable on subprocess A before
    //     we kill it. If this fails, the bug is in the harness/API, not
    //     in restart persistence.
    let res = h
        .client()
        .get(format!(
            "{}/v1/sessions?tenant_id={}&workspace_id={}&project_id={}",
            h.base_url, h.tenant, h.workspace, h.project,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("sessions list reaches subprocess A");
    let body_a: serde_json::Value = res.json().await.expect("list json A");
    let items_a = body_a
        .as_array()
        .cloned()
        .or_else(|| body_a.get("items").and_then(|v| v.as_array()).cloned())
        .expect("sessions list body A is array or {items: [..]}");
    let found_a = items_a
        .iter()
        .any(|s| s.get("session_id").and_then(|v| v.as_str()) == Some(session_id.as_str()));
    assert!(
        found_a,
        "session {session_id} must be visible on A before restart: {body_a}",
    );

    // 3. SIGKILL subprocess A and spawn subprocess B on the same port,
    //    same SQLite file, same admin token.
    h.sigkill_and_restart()
        .await
        .expect("sigkill+restart succeeds");

    // 4. Subprocess B is live and serving /health — prove it's actually
    //    serving (not just that the port is open).
    let ready = h.poll_readiness_until_ready(Duration::from_secs(5)).await;
    assert!(ready, "subprocess B did not become ready within 5s");

    let res = h
        .client()
        .get(format!("{}/health", h.base_url))
        .send()
        .await
        .expect("health reaches subprocess B");
    assert!(
        res.status().is_success(),
        "subprocess B /health: {}",
        res.status()
    );

    // 5. Projection read-back against subprocess B: the session created
    //    on A must be visible on B — proving the event log survived the
    //    subprocess death and was replayed into B's in-memory projections.
    let res = h
        .client()
        .get(format!(
            "{}/v1/sessions?tenant_id={}&workspace_id={}&project_id={}",
            h.base_url, h.tenant, h.workspace, h.project,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("sessions list reaches subprocess B");
    assert_eq!(
        res.status().as_u16(),
        200,
        "sessions list on B: {}",
        res.text().await.unwrap_or_default(),
    );
    let body: serde_json::Value = res.json().await.expect("list json");
    let items = body
        .as_array()
        .cloned()
        .or_else(|| body.get("items").and_then(|v| v.as_array()).cloned())
        .expect("sessions list body is array or {items: [..]}");
    let found = items
        .iter()
        .any(|s| s.get("session_id").and_then(|v| v.as_str()) == Some(session_id.as_str()));
    assert!(
        found,
        "session {session_id} from A must be visible on B (DB survived subprocess restart): {body}",
    );
}
