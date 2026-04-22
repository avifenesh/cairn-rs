//! RFC 020 recovery integration tests — compliance evidence for the
//! `RecoveryServiceImpl` durability invariants.
//!
//! Every test in this file:
//!
//! 1. Boots a real `cairn-app` subprocess via [`LiveHarness`] with a
//!    per-test SQLite event log so state survives process restart.
//! 2. Drives cairn through its public HTTP surface only — no in-process
//!    shortcuts. This is load-bearing: only a real `SIGKILL` + respawn
//!    cycle is admissible proof for RFC 020 invariants; in-memory
//!    "simulated restarts" don't exercise the boot-time event-log
//!    replay or the readiness gate.
//! 3. Asserts against the secondary event log (`GET /v1/events`), not
//!    against transient in-memory views. The event log is the durable
//!    source of truth that `RecoveryService` itself reads on boot.
//!
//! Test → RFC 020 invariant mapping:
//!
//! | RFC 020 # | Test fn                                                     |
//! |-----------|-------------------------------------------------------------|
//! | #1        | `clean_crash_recovery_restores_non_terminal_runs`           |
//! | #6        | `in_flight_approval_survives_crash`                         |
//! | #11       | `recovery_summary_emitted_once_per_boot`                    |
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

    create_session(&h, &session_id).await;
    create_run(&h, &session_id, &run_id).await;
    // Request an (unresolved) approval against the run. The approval
    // service's `request_with_context` batches `ApprovalRequested` +
    // `RunStateChanged(Pending → WaitingApproval)` into a single
    // `EventLog::append`, so both land in the SQLite secondary log and
    // survive the crash. The recovery sweep only enumerates non-terminal
    // runs excluding `Pending`, so this step is what actually exercises
    // the matrix when we restart.
    request_approval(&h, &run_id, &approval_id).await;

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

// ─────────────────────────────────────────────────────────────────────────────
// RFC 020 Integration Test #6 — in-flight approval survives crash.
// ─────────────────────────────────────────────────────────────────────────────

/// RFC 020 Integration Test #6: a pending approval anchored to a run
/// survives a crash. After restart the approval row is still present in
/// `ApprovalReadModel` with no decision attached, and `RecoveryService`
/// emits the "approval still pending; waiting for operator" matrix case
/// — i.e. it does NOT synthesise an `ApprovalResolved` event.
///
/// What the test does:
///
/// 1. Spin up a real `cairn-app` subprocess with per-test SQLite event log.
/// 2. Create session + run, then `POST /v1/approvals` — the approval
///    service batches `ApprovalRequested` + `RunStateChanged(Pending →
///    WaitingApproval)` into one `EventLog::append`, so both land durably
///    in the secondary SQLite log.
/// 3. Read back the pre-crash approval list and confirm it is present and
///    unresolved. Snapshot the event log.
/// 4. SIGKILL + restart against the same SQLite file.
/// 5. Re-query the approval list: the approval must still be present and
///    unresolved. Diff the event log against the pre-crash snapshot: the
///    new events must NOT include an `approval_resolved` — recovery is
///    advisory in this matrix cell — but must include the `recovery_*`
///    advisory pair.
///
/// Why we don't assert on `run.state` here: `GET /v1/runs/:id` routes
/// through `state.runtime.runs.get`, which in production delegates to
/// the Fabric run-state adapter. `ApprovalService::request_with_context`
/// appends the batched `RunStateChanged` directly to the secondary event
/// log, which is the durable source of truth that `RecoveryService`
/// reads from, but cairn's fabric-adapted run view doesn't observe that
/// side-channel write. Asserting on the event log keeps the compliance
/// claim anchored to the RFC 020 durability invariant — the log, not a
/// non-durable in-memory view — and avoids conflating #6 (approval
/// survival) with a separate and pre-existing fabric/store read-view
/// coherence question.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_flight_approval_survives_crash() {
    let mut h = LiveHarness::setup_with_sqlite().await;

    let session_id = format!("sess_{}", h.project);
    let run_id = format!("run_{}", h.project);
    let approval_id = format!("appr_{}", h.project);

    create_session(&h, &session_id).await;
    create_run(&h, &session_id, &run_id).await;
    request_approval(&h, &run_id, &approval_id).await;

    // Pre-crash sanity: approval exists and is unresolved; the event log
    // already carries the state transition that recovery will observe.
    assert!(
        approval_pending(&h, &approval_id).await,
        "pre-crash approval should be present and unresolved",
    );
    let events_before = fetch_event_log(&h).await;
    assert!(
        events_before
            .iter()
            .any(|e| event_type(e) == "approval_requested"),
        "approval_requested missing from pre-crash event log: {events_before:#?}",
    );
    assert!(
        events_before
            .iter()
            .any(|e| event_type(e) == "run_state_changed"),
        "run_state_changed(Pending->WaitingApproval) missing from pre-crash event log: {events_before:#?}",
    );

    // ── Crash + restart ─────────────────────────────────────────────────
    h.sigkill_and_restart()
        .await
        .expect("sigkill + restart against the same SQLite file");

    // Post-crash: approval still present and still unresolved.
    assert!(
        approval_pending(&h, &approval_id).await,
        "post-crash approval should still be present and unresolved",
    );

    // Post-crash event-log delta: recovery must NOT synthesise an
    // `approval_resolved` (the matrix cell is "state unchanged; recovery
    // advisory only"), and it MUST emit the advisory
    // recovery_attempted/recovery_completed pair.
    let events_after = fetch_event_log(&h).await;
    let new_events: Vec<&Value> = events_after
        .iter()
        .filter(|e| !events_before.iter().any(|b| position(b) == position(e)))
        .collect();
    assert!(
        !new_events
            .iter()
            .any(|e| event_type(e) == "approval_resolved"),
        "recovery must not auto-resolve a pending approval; new events: {new_events:#?}",
    );
    assert!(
        new_events
            .iter()
            .any(|e| event_type(e) == "recovery_attempted"),
        "expected recovery_attempted after restart; new events: {new_events:#?}",
    );
    assert!(
        new_events
            .iter()
            .any(|e| event_type(e) == "recovery_completed"),
        "expected recovery_completed after restart; new events: {new_events:#?}",
    );
    // The "approval still pending" matrix cell specifically emits NO new
    // `run_state_changed` — the recovery is advisory only.
    assert!(
        !new_events
            .iter()
            .any(|e| event_type(e) == "run_state_changed"),
        "recovery emitted an unexpected run_state_changed for a pending approval: {new_events:#?}",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// RFC 020 Integration Test #11 — recovery summary emitted once per boot.
// ─────────────────────────────────────────────────────────────────────────────

/// RFC 020 Integration Test #11: each boot's `RecoveryService::recover_all`
/// runs exactly once and emits exactly one `RecoveryAttempted` +
/// `RecoveryCompleted` pair per recoverable run. A subsequent restart runs
/// the sweep again, doubling the counts. The first (pre-crash) boot has
/// no non-`Pending` runs to scan, so it emits zero recovery events.
///
/// Note on `boot_id`: `RecoveryAttempted`/`RecoveryCompleted` carry a
/// `boot_id` payload field, but the `/v1/events` endpoint only surfaces
/// `event_type` summaries — payloads are not currently wire-exposed.
/// This test therefore proves "one sweep per boot" by counting event
/// pairs across two restarts, not by reading `boot_id` directly. The
/// per-boot uniqueness of `boot_id` is proved at the service level by
/// `RecoveryServiceImpl::recover_all`'s signature (it takes `&BootId`)
/// and by `main.rs` generating a fresh `BootId::new(Uuid::now_v7())` on
/// every startup.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_summary_emitted_once_per_boot() {
    let mut h = LiveHarness::setup_with_sqlite().await;

    let session_id = format!("sess_{}", h.project);
    let run_id = format!("run_{}", h.project);
    let approval_id = format!("appr_{}", h.project);

    create_session(&h, &session_id).await;
    create_run(&h, &session_id, &run_id).await;
    request_approval(&h, &run_id, &approval_id).await;

    // Boot 1 baseline: no recovery events yet. `main.rs` runs
    // `RecoveryService::recover_all` on every boot, but on boot 1 the
    // only run is `Pending → WaitingApproval` which only reached
    // `WaitingApproval` *after* the recovery sweep completed, so no
    // `recovery_attempted` event targeted it.
    let events_boot1 = fetch_event_log(&h).await;
    assert_eq!(
        count_event(&events_boot1, "recovery_attempted"),
        0,
        "boot 1 should emit no recovery_attempted; events={events_boot1:#?}",
    );
    assert_eq!(
        count_event(&events_boot1, "recovery_completed"),
        0,
        "boot 1 should emit no recovery_completed; events={events_boot1:#?}",
    );

    // ── Restart #1 ──────────────────────────────────────────────────────
    h.sigkill_and_restart()
        .await
        .expect("sigkill + restart #1 against the same SQLite file");

    let events_boot2 = fetch_event_log(&h).await;
    // One recoverable run → exactly one attempted + one completed.
    assert_eq!(
        count_event(&events_boot2, "recovery_attempted"),
        1,
        "boot 2 should emit exactly one recovery_attempted; events={events_boot2:#?}",
    );
    assert_eq!(
        count_event(&events_boot2, "recovery_completed"),
        1,
        "boot 2 should emit exactly one recovery_completed; events={events_boot2:#?}",
    );

    // ── Restart #2 ──────────────────────────────────────────────────────
    // Proves the sweep fires once *per boot*, not once total. If
    // RecoveryService were somehow guarded to only run on first startup,
    // this assertion would fail.
    h.sigkill_and_restart()
        .await
        .expect("sigkill + restart #2 against the same SQLite file");

    let events_boot3 = fetch_event_log(&h).await;
    assert_eq!(
        count_event(&events_boot3, "recovery_attempted"),
        2,
        "boot 3 should have cumulative 2 recovery_attempted; events={events_boot3:#?}",
    );
    assert_eq!(
        count_event(&events_boot3, "recovery_completed"),
        2,
        "boot 3 should have cumulative 2 recovery_completed; events={events_boot3:#?}",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers shared by #6 and #11.
// ─────────────────────────────────────────────────────────────────────────────

async fn create_session(h: &LiveHarness, session_id: &str) {
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
}

async fn create_run(h: &LiveHarness, session_id: &str, run_id: &str) {
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

    // Wait for the `RunCreated` event to propagate from `FabricRunService`
    // via `EventBridge` into the `InMemoryStore` run projection. Without
    // this, a tightly-following `POST /v1/approvals` may race and hit
    // `ApprovalServiceImpl::request_with_context`'s `RunReadModel::get`
    // before the projection is populated, yielding a 404 "run not found".
    // The race is cross-test-flaky because the shared Valkey testcontainer
    // serialises fabric appends across every parallel test in the binary.
    wait_for_run_projected(h, run_id).await;
}

/// Poll `/v1/runs/:id` until the run is visible in the projection (or panic
/// after 3s). Covers the eventbridge-populates-InMemoryStore delay.
async fn wait_for_run_projected(h: &LiveHarness, run_id: &str) {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let res = h
            .client()
            .get(format!("{}/v1/runs/{}", h.base_url, run_id))
            .bearer_auth(&h.admin_token)
            .header("X-Cairn-Tenant", &h.tenant)
            .header("X-Cairn-Workspace", &h.workspace)
            .header("X-Cairn-Project", &h.project)
            .send()
            .await
            .expect("GET /v1/runs/:id reached server");
        if res.status().is_success() {
            return;
        }
        if Instant::now() > deadline {
            panic!(
                "run {run_id} never became visible in projection within 3s (last status {})",
                res.status(),
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn request_approval(h: &LiveHarness, run_id: &str, approval_id: &str) {
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
}

async fn approval_pending(h: &LiveHarness, approval_id: &str) -> bool {
    let res = h
        .client()
        .get(format!("{}/v1/approvals", h.base_url))
        .query(&[
            ("tenant_id", &h.tenant),
            ("workspace_id", &h.workspace),
            ("project_id", &h.project),
        ])
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/approvals reached server");
    assert!(
        res.status().is_success(),
        "GET /v1/approvals: {}",
        res.status(),
    );
    let body: Value = res.json().await.expect("approvals body is json");
    let items = body
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no items in /v1/approvals body: {body}"));

    items.iter().any(|a| {
        let id_matches = a
            .get("approval_id")
            .and_then(Value::as_str)
            .map(|s| s == approval_id)
            .unwrap_or(false);
        if !id_matches {
            return false;
        }
        // Unresolved approvals carry `decision: null` (serde default for
        // `Option::None`) or, on some wire paths, an explicit "pending".
        // Resolved approvals carry `decision: "approved" | "denied" | ...`.
        match a.get("decision") {
            None => true,
            Some(Value::Null) => true,
            Some(Value::String(s)) => s == "pending",
            _ => false,
        }
    })
}

fn count_event(events: &[Value], kind: &str) -> usize {
    events.iter().filter(|e| event_type(e) == kind).count()
}
