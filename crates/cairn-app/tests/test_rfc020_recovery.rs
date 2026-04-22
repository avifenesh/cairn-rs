//! RFC 020 Integration Test #1 — clean crash recovery restores non-terminal runs.
//!
//! Compliance evidence for RFC 020 Track 1 (`RecoveryServiceImpl`). Per the
//! user rule `feedback_integration_tests_only.md`, unit tests on recovery
//! types do not count — this file is the end-to-end proof.
//!
//! What the test does:
//!
//! 1. Spin up a real `cairn-app` subprocess with a per-test SQLite event log
//!    via [`LiveHarness::setup_with_sqlite`].
//! 2. Create a session + run, then request an approval against the run —
//!    this transitions the run from `Pending` to `WaitingApproval` via
//!    `ApprovalService`, and the events land in the SQLite event log (the
//!    approval path goes through `EventLog::append` / the secondary log).
//! 3. SIGKILL the subprocess and respawn it on the same SQLite file via
//!    [`LiveHarness::sigkill_and_restart`]. The subprocess boot sequence
//!    runs `SandboxService::recover_all` → `RecoveryService::recover_all`
//!    → readiness gate flip. If recovery fails, `/health/ready` stays 503
//!    and the harness panics.
//! 4. Assert the SQLite event log now contains a pair of
//!    `recovery_attempted` + `recovery_completed` events emitted by the
//!    second boot's `RecoveryService`, proving the sweep actually ran.
//!
//! Why SQLite and not Postgres: the CI box doesn't provide Postgres. The
//! recovery service's SQL is standard SQL-92; cairn-store's pg and sqlite
//! adapters share the same `RunReadModel` / `CheckpointReadModel` /
//! `ApprovalReadModel` shape, so a passing test on SQLite pins the logic
//! on Postgres by construction. The PR body carries the portability grep.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clean_crash_recovery_restores_non_terminal_runs() {
    // ── Boot 1: create a run and push it to WaitingApproval ──────────────
    let mut h = LiveHarness::setup_with_sqlite().await;

    let session_id = format!("sess_{}", h.project);
    let run_id = format!("run_{}", h.project);
    let approval_id = format!("appr_{}", h.project);

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
        .expect("POST /v1/sessions reached server");
    assert!(
        res.status().is_success(),
        "POST /v1/sessions: {} body={}",
        res.status(),
        res.text().await.unwrap_or_default(),
    );

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
        .expect("POST /v1/runs reached server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "POST /v1/runs: {} body={}",
        res.status(),
        res.text().await.unwrap_or_default(),
    );

    // Request an (unresolved) approval against the run. The approval
    // service's `request_with_context` batches `ApprovalRequested` +
    // `RunStateChanged(Pending → WaitingApproval)` into a single
    // `EventLog::append`, so both land in the SQLite secondary log and
    // survive the crash. The recovery sweep only enumerates non-terminal
    // runs excluding `Pending`, so this step is what actually exercises
    // the matrix when we restart.
    let res = h
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
        .expect("POST /v1/approvals reached server");
    assert!(
        res.status().is_success(),
        "POST /v1/approvals: {} body={}",
        res.status(),
        res.text().await.unwrap_or_default(),
    );

    // Boot-1 event log baseline. We read through `/v1/events` (admin-only)
    // so we can diff against the boot-2 log after restart.
    let events_before = fetch_event_log(&h).await;
    assert!(
        events_before
            .iter()
            .any(|e| event_type(e) == "approval_requested"),
        "approval_requested not in event log before crash: {events_before:#?}",
    );
    assert!(
        events_before
            .iter()
            .any(|e| event_type(e) == "run_state_changed"),
        "run_state_changed not in event log before crash: {events_before:#?}",
    );

    // ── Crash + restart ─────────────────────────────────────────────────
    h.sigkill_and_restart()
        .await
        .expect("sigkill + restart against the same SQLite file");

    // The harness's `restart()` already polled `/health/ready` → 200. If
    // `RecoveryService::recover_all` had returned `Err`, `main.rs` would
    // have `std::process::exit(1)` and the banner scrape would have
    // failed — so reaching this line already means recovery succeeded.

    // ── Assert recovery events landed in the event log ───────────────────
    let events_after = fetch_event_log(&h).await;

    let new_events: Vec<&Value> = events_after
        .iter()
        .filter(|e| !events_before.iter().any(|b| position(b) == position(e)))
        .collect();

    let recovery_attempted = new_events
        .iter()
        .filter(|e| event_type(e) == "recovery_attempted")
        .count();
    let recovery_completed = new_events
        .iter()
        .filter(|e| event_type(e) == "recovery_completed")
        .count();

    assert!(
        recovery_attempted >= 1,
        "expected >= 1 recovery_attempted after restart; new events: {new_events:#?}",
    );
    assert!(
        recovery_completed >= 1,
        "expected >= 1 recovery_completed after restart; new events: {new_events:#?}",
    );
    // Matrix for `WaitingApproval` with no resolved approval: "state
    // unchanged, advisory only". No extra `run_state_changed` should be
    // emitted.
    assert!(
        !new_events
            .iter()
            .any(|e| event_type(e) == "run_state_changed"),
        "recovery emitted an unexpected run_state_changed: {new_events:#?}",
    );
}

/// Fetch the server's event log via the admin endpoint. The endpoint
/// returns either a bare JSON array or `{ "items": [...] }`; both shapes
/// have appeared in the codebase so the helper accepts either.
async fn fetch_event_log(h: &LiveHarness) -> Vec<Value> {
    let res = h
        .client()
        .get(format!("{}/v1/events?limit=200", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/events reached server");
    assert!(
        res.status().is_success(),
        "GET /v1/events: {}",
        res.status()
    );
    let body: Value = res.json().await.expect("event log body is json");
    if let Some(arr) = body.as_array() {
        arr.clone()
    } else if let Some(arr) = body.get("items").and_then(Value::as_array) {
        arr.clone()
    } else {
        panic!("unexpected /v1/events body shape: {body}");
    }
}

fn event_type(ev: &Value) -> &str {
    ev.get("event_type")
        .or_else(|| ev.get("event"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn position(ev: &Value) -> i64 {
    ev.get("position").and_then(Value::as_i64).unwrap_or(-1)
}
