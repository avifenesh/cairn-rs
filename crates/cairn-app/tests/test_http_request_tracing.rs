//! Request tracing: an incoming `x-trace-id` header must flow through
//! the runtime so that events emitted during the request are tagged
//! with that trace id, and `GET /v1/trace/:trace_id` must surface
//! those events back to the caller.
//!
//! Covers task #120: request_tracing migration. The in-memory fixture
//! couldn't satisfy `runs.start_command` once it became a real Fabric
//! mutation, so the test was dropped in PR #67. This reinstates it
//! against a live server.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn trace_id_header_threads_through_to_event_stream() {
    let h = LiveHarness::setup().await;
    let session_id = format!("sess_{}", h.project);
    let run_id = format!("run_{}", h.project);
    let trace_id = format!("trace-{}-{}", h.project, uuid::Uuid::new_v4().simple());

    // Seed a session so the run has somewhere to live.
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
    assert_eq!(r.status().as_u16(), 201);

    // Create a run with an explicit x-trace-id header. The request_id
    // middleware picks the header up, sets the thread-local, and every
    // event minted via make_envelope() within the handler inherits it
    // as correlation_id.
    let res = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .header("x-trace-id", &trace_id)
        .json(&json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
            "session_id": session_id,
            "run_id": run_id,
        }))
        .send()
        .await
        .expect("POST /v1/runs reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "run create: {}",
        res.text().await.unwrap_or_default(),
    );

    // Server must echo the trace id back on the response header — that's
    // the observable promise of the middleware.
    let echoed = res
        .headers()
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .expect("response carries x-trace-id header");
    assert_eq!(echoed, trace_id, "x-trace-id header round-trip");

    // Fetch the trace view. The run-creation event stream for this
    // trace id must be non-empty; the exact event_type labels are
    // implementation details but at least one span must reference the
    // run_id we just created.
    let res = h
        .client()
        .get(format!("{}/v1/trace/{}", h.base_url, trace_id))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/trace/:id reaches server");
    assert_eq!(res.status().as_u16(), 200, "trace read");
    let view: serde_json::Value = res.json().await.expect("trace json");
    assert_eq!(
        view["trace_id"].as_str(),
        Some(trace_id.as_str()),
        "trace view echoes id"
    );
    let spans = view["spans"]
        .as_array()
        .cloned()
        .expect("spans array present");
    assert!(
        !spans.is_empty(),
        "trace must contain at least one span for the run creation: {}",
        view,
    );
    let refs_run = spans.iter().any(|s| {
        s.get("entity_id")
            .and_then(|v| v.as_str())
            .map(|eid| eid.contains(&run_id))
            .unwrap_or(false)
    });
    assert!(
        refs_run,
        "no span references the created run_id {}: {}",
        run_id, view,
    );
}
