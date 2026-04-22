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

    // 3. Give cairn-app's SQLite dual-writer a beat to flush the WAL to
    //    disk before we SIGKILL. cairn-app uses sqlx's SQLite defaults
    //    (WAL journaling, synchronous=NORMAL), which acks commits before
    //    a full fsync — a SIGKILL inside that window can lose the last
    //    write. 500 ms is well beyond any realistic flush latency while
    //    still being a fraction of the test budget. An RFC 020 crash test
    //    that exercises a longer mutation sequence would naturally have
    //    this gap in its own timeline.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 4. SIGKILL subprocess A and spawn subprocess B on the same port,
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
    //
    //    The replay runs synchronously during startup, but the session's
    //    runtime registration (via FabricServices) may land a tick later
    //    than /health/ready reports. Poll for up to 5s rather than assume
    //    instantaneous availability — this is still well below any timeout
    //    a real integration test would accept.
    let list_url = format!(
        "{}/v1/sessions?tenant_id={}&workspace_id={}&project_id={}",
        h.base_url, h.tenant, h.workspace, h.project,
    );
    let mut last_body = serde_json::Value::Null;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut found = false;
    while tokio::time::Instant::now() < deadline {
        let res = h
            .client()
            .get(&list_url)
            .bearer_auth(&h.admin_token)
            .send()
            .await
            .expect("sessions list reaches subprocess B");
        assert_eq!(
            res.status().as_u16(),
            200,
            "sessions list on B non-200: {}",
            res.text().await.unwrap_or_default(),
        );
        last_body = res.json().await.expect("list json B");
        let items = last_body
            .as_array()
            .cloned()
            .or_else(|| last_body.get("items").and_then(|v| v.as_array()).cloned())
            .expect("sessions list body B is array or {items: [..]}");
        if items
            .iter()
            .any(|s| s.get("session_id").and_then(|v| v.as_str()) == Some(session_id.as_str()))
        {
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        found,
        "session {session_id} from A must be visible on B within 5s (DB survived subprocess restart): {last_body}",
    );
}
