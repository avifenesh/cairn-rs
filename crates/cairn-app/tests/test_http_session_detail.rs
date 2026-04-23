//! Regression coverage for #170: SessionDetailPage now loads runs via
//! `GET /v1/sessions/:id/runs`, which applies session filtering
//! server-side instead of relying on a client-side `session_id` filter
//! applied on top of a globally capped `/v1/runs?limit=500` response
//! (the original path silently dropped runs once a project passed 500
//! total runs).
//!
//! This test verifies that the endpoint returns *every* run created in
//! the requested session and does not mix in runs from a sibling
//! session under the same project scope.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn list_session_runs_returns_all_runs_unfiltered() {
    let h = LiveHarness::setup().await;

    let session_id = format!("sess_{}", &h.project);
    let other_session_id = format!("sess_other_{}", &h.project);

    // Create the primary session.
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
        .expect("POST /v1/sessions reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "session create: {}",
        res.text().await.unwrap_or_default()
    );

    // Create a sibling session in the same project — its runs must not
    // leak into the target session's list.
    let res = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
            "session_id": other_session_id,
        }))
        .send()
        .await
        .expect("POST sibling session reaches server");
    assert_eq!(res.status().as_u16(), 201, "sibling session create");

    // 5 runs under the target session.
    let mut expected_runs: Vec<String> = Vec::with_capacity(5);
    for i in 0..5 {
        let run_id = format!("run_{}_{i}", &h.project);
        let res = h
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
            .expect("POST /v1/runs reaches server");
        assert_eq!(
            res.status().as_u16(),
            201,
            "run create #{i}: {}",
            res.text().await.unwrap_or_default()
        );
        expected_runs.push(run_id);
    }

    // 2 runs under the sibling session. These must not appear in the
    // target session's list.
    for i in 0..2 {
        let run_id = format!("run_other_{}_{i}", &h.project);
        let res = h
            .client()
            .post(format!("{}/v1/runs", h.base_url))
            .bearer_auth(&h.admin_token)
            .json(&json!({
                "tenant_id": h.tenant,
                "workspace_id": h.workspace,
                "project_id": h.project,
                "session_id": other_session_id,
                "run_id": run_id,
            }))
            .send()
            .await
            .expect("POST sibling run reaches server");
        assert_eq!(res.status().as_u16(), 201, "sibling run create #{i}");
    }

    // GET /v1/sessions/:id/runs — all 5 target runs, zero sibling runs.
    // Build the URL via `reqwest::Url::path_segments_mut` so the
    // `session_id` is percent-encoded; validation in cairn-app only
    // rejects control chars, so `/` and other reserved chars are
    // legal session ids and must not break routing.
    let mut url = reqwest::Url::parse(&format!("{}/", h.base_url))
        .expect("base_url parses as URL");
    {
        let mut segments = url
            .path_segments_mut()
            .expect("base_url supports path segments");
        segments.push("v1");
        segments.push("sessions");
        segments.push(&session_id);
        segments.push("runs");
    }
    url.query_pairs_mut().append_pair("limit", "500");

    let res = h
        .client()
        .get(url)
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/sessions/:id/runs reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "list session runs: {}",
        res.text().await.unwrap_or_default()
    );

    let body: serde_json::Value = res.json().await.expect("list json");
    let items = body
        .as_array()
        .cloned()
        .or_else(|| body.get("items").and_then(|v| v.as_array()).cloned())
        .expect("list body is array or {items: [..]}");

    let returned_ids: Vec<String> = items
        .iter()
        .filter_map(|v| v.get("run_id").and_then(|x| x.as_str()).map(String::from))
        .collect();

    assert_eq!(
        returned_ids.len(),
        5,
        "session {session_id} should return 5 runs, got {}: {returned_ids:?}",
        returned_ids.len()
    );
    for expected in &expected_runs {
        assert!(
            returned_ids.contains(expected),
            "expected run {expected} missing from session list: {returned_ids:?}",
        );
    }
    for rid in &returned_ids {
        assert!(
            !rid.starts_with("run_other_"),
            "sibling run {rid} leaked into session {session_id}'s list",
        );
    }
}

/// Unknown session id → 404, not an empty array.
#[tokio::test]
async fn list_session_runs_unknown_session_returns_404() {
    let h = LiveHarness::setup().await;
    let res = h
        .client()
        .get(format!("{}/v1/sessions/does-not-exist/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET unknown session runs reaches server");
    assert_eq!(
        res.status().as_u16(),
        404,
        "unknown session should 404: {}",
        res.text().await.unwrap_or_default()
    );
}
