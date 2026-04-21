//! Concurrent claim race: N workers hammer `POST /v1/tasks/{id}/claim`
//! simultaneously. FF's scheduler-routed claim must grant the lease to
//! exactly one caller; every other caller must receive a typed error
//! (non-success HTTP status).
//!
//! Covers the lease-exclusivity invariant that regressed silently under
//! the in-memory runtime (which didn't enforce atomic claim) and is now
//! the responsibility of FF's scheduler.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::json;
use support::live_fabric::LiveHarness;

const WORKER_COUNT: usize = 6;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn only_one_worker_wins_a_concurrent_claim() {
    let h = LiveHarness::setup().await;
    let session_id = format!("sess_{}", h.project);
    let run_id = format!("run_{}", h.project);
    let task_id = format!("task_{}", h.project);

    // Set up session + run + task (sequential — that part isn't under test).
    seed_session_run_task(&h, &session_id, &run_id, &task_id).await;

    // Fire N claim requests concurrently. Each in its own tokio task so
    // they race through the HTTP client pool onto the server at the same
    // moment. Use `tokio::join!`-style fan-out via `futures::join_all`.
    let successes = Arc::new(AtomicUsize::new(0));
    let rejections = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::with_capacity(WORKER_COUNT);
    for i in 0..WORKER_COUNT {
        let client = h.client().clone();
        let url = format!("{}/v1/tasks/{}/claim", h.base_url, task_id);
        let token = h.admin_token.clone();
        let worker_id = format!("worker-{i}-{}", h.project);
        let successes = successes.clone();
        let rejections = rejections.clone();
        handles.push(tokio::spawn(async move {
            let res = client
                .post(&url)
                .bearer_auth(&token)
                .json(&json!({
                    "worker_id": worker_id,
                    "lease_duration_ms": 60_000,
                }))
                .send()
                .await
                .expect("claim request reaches server");
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            if status.is_success() {
                successes.fetch_add(1, Ordering::SeqCst);
                Some((worker_id, body))
            } else {
                rejections.fetch_add(1, Ordering::SeqCst);
                // Must be a typed error, not a 500.
                assert!(
                    status.as_u16() < 500,
                    "claim race produced a 5xx: {status} {body}",
                );
                None
            }
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let winners: Vec<_> = results
        .into_iter()
        .filter_map(|r| r.expect("claim task didn't panic"))
        .collect();

    assert_eq!(
        successes.load(Ordering::SeqCst),
        1,
        "exactly one claim must succeed, got {} winners + {} rejections",
        successes.load(Ordering::SeqCst),
        rejections.load(Ordering::SeqCst),
    );
    assert_eq!(
        rejections.load(Ordering::SeqCst),
        WORKER_COUNT - 1,
        "every other claim must be rejected",
    );
    assert_eq!(winners.len(), 1, "only one winner payload");
}

async fn seed_session_run_task(h: &LiveHarness, session_id: &str, run_id: &str, task_id: &str) {
    let body = |extra: serde_json::Value| {
        let mut obj = json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
        });
        for (k, v) in extra.as_object().expect("object").iter() {
            obj[k] = v.clone();
        }
        obj
    };

    let s = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&body(json!({ "session_id": session_id })))
        .send()
        .await
        .expect("session create reaches server");
    assert_eq!(s.status().as_u16(), 201, "session seed");

    let r = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&body(json!({ "session_id": session_id, "run_id": run_id })))
        .send()
        .await
        .expect("run create reaches server");
    assert_eq!(r.status().as_u16(), 201, "run seed");

    let t = h
        .client()
        .post(format!("{}/v1/tasks", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&body(
            json!({ "task_id": task_id, "parent_run_id": run_id }),
        ))
        .send()
        .await
        .expect("task create reaches server");
    assert_eq!(t.status().as_u16(), 201, "task seed");
}
