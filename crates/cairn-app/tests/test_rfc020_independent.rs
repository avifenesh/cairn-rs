//! RFC 020 compliance integration tests that have no Track-level blocker.
//!
//! Ships two integration tests from the RFC 020 §"Integration Tests
//! (Compliance Proof)" list that can be expressed independently of the
//! RecoveryService work in Track 1:
//!
//! * **Test #7** — "Decision cache survives: cache a decision (RFC 019),
//!   restart; confirm the decision is available; a subsequent equivalent
//!   request returns via cache hit without re-nudging."
//!
//! * **Test #12** — "Postgres required for team mode: attempt to start
//!   cairn-app in team mode with a SQLite DB; confirm startup refuses
//!   with a clear error message (not just a warning)."
//!
//! Per project policy (`feedback_integration_tests_only`), both tests
//! drive a real `cairn-app` subprocess. Test #7 uses `LiveHarness` so it
//! can sigkill+restart the running process. Test #12 spawns the binary
//! directly via `tokio::process::Command` because it is testing *refusal
//! to start* — LiveHarness would try to scrape a listening banner that
//! never gets printed.

mod support;

use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

use support::live_fabric::LiveHarness;

// ── Test #7: decision cache survives restart ─────────────────────────────────

/// RFC 020 Integration Test #7.
///
/// The exact contract from RFC 020: cache a decision, restart, confirm the
/// decision is available, and an equivalent request returns via cache hit
/// without re-nudging.
///
/// This test is `#[ignore]`d as of this commit because the observable surface
/// needed to drive it end-to-end from HTTP does not yet exist:
///
/// 1. There is no `POST /v1/decisions` evaluate endpoint on cairn-app. The
///    only decision-cache-writing path today is the in-process orchestrator
///    calling `DecisionService::evaluate()`, which integration tests cannot
///    trigger without running a full tool-invoking run.
///
/// 2. `DecisionServiceImpl::cache` is an in-process `Mutex<HashMap>` with no
///    event-log persistence and no startup replay. On restart the map is
///    empty — so even if we could seed a decision pre-restart, the post-
///    restart lookup would return `Miss`, not `CacheHit`.
///
/// 3. `main.rs` marks `readiness.decision_cache = complete(0)` with a comment
///    that real per-branch recovery "is handled by a later track", confirming
///    the feature is deferred.
///
/// Un-ignore this test when BOTH of the following land:
///   (a) a cairn-app HTTP surface that observably writes to the decision
///       cache (e.g. a `POST /v1/decisions/evaluate` endpoint, or a run
///       path the test can invoke that exercises `DecisionService::evaluate`
///       end-to-end and exposes the resulting cache state via
///       `GET /v1/decisions/cache`);
///   (b) `DecisionRecorded` event persistence + startup replay so the cache
///       is rebuilt from the event log.
///
/// The test body below is written so that un-ignoring will exercise the
/// contract once those preconditions are met; until then, it documents
/// the gap as an explicit `#[ignore] + reason` rather than hiding it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "decision cache has no HTTP write-path + no event-log replay yet; \
            see RFC 020 §'Decision cache' and main.rs readiness stub. \
            As-of commit 48e63f6 startup marks decision_cache complete(0) \
            without actually rebuilding any cache state from the event log."]
async fn decision_cache_survives_restart() {
    // Boot with SQLite persistence so the event log (once wired to persist
    // DecisionRecorded) actually survives sigkill.
    let mut harness = LiveHarness::setup_with_sqlite().await;

    // ── Pre-restart: seed a cacheable decision ────────────────────────────
    // Placeholder for when the evaluate endpoint lands. The expected shape:
    //   POST /v1/decisions/evaluate
    //   Body: { "kind": { "ToolInvocation": { "tool_name": "grep_search",
    //                                          "effect": "Observational" } },
    //           ... }
    // Response should echo the decision_id + outcome. Observational tool
    // invocations have CachePolicy::AlwaysCache + 86400s TTL so the first
    // call always produces a cacheable entry.
    //
    // Until the endpoint exists, assert that the endpoint is indeed absent
    // so that when a future PR adds it, this test starts failing here and
    // forces us to un-ignore + fill in the seeding step.
    let probe = harness
        .client()
        .post(format!("{}/v1/decisions/evaluate", harness.base_url))
        .bearer_auth(&harness.admin_token)
        .json(&serde_json::json!({
            "kind": { "ToolInvocation": { "tool_name": "grep_search",
                                          "effect": "Observational" } }
        }))
        .send()
        .await
        .expect("probe reaches server");
    // Accept 404 (no such route) OR 405 (route exists for GET /v1/decisions
    // but POST on it is not allowed). Both mean "no evaluate endpoint".
    // When a real `POST /v1/decisions/evaluate` lands, the server will
    // return 2xx and this assertion will fire, telling whoever is un-
    // ignoring the test to finish wiring the post-restart cache-hit path.
    let status = probe.status().as_u16();
    assert!(
        matches!(status, 404 | 405),
        "POST /v1/decisions/evaluate unexpectedly returned {status} — the \
         endpoint appears to exist. Un-ignore this test and finish wiring \
         the post-restart cache-hit assertion."
    );

    // ── Restart ───────────────────────────────────────────────────────────
    harness
        .sigkill_and_restart()
        .await
        .expect("sigkill + restart succeeds");

    // ── Post-restart: equivalent request returns via cache hit ────────────
    // When the evaluate endpoint exists, this second call against the
    // restarted subprocess must:
    //   * succeed (200),
    //   * produce the same `outcome`,
    //   * expose a `source: "CacheHit"` marker (or equivalent) indicating
    //     no fresh evaluation was needed,
    //   * and `GET /v1/decisions/cache` should list the decision with a
    //     non-zero `hit_count`.
    //
    // Intentionally left un-finished — see #[ignore] reason above.
}

// ── Test #12: team mode refuses SQLite ───────────────────────────────────────

/// RFC 020 Integration Test #12.
///
/// Attempts to boot cairn-app in `--mode team` with a SQLite database.
/// Expects the subprocess to refuse to start with a nonzero exit code and
/// a clear, operator-readable error mentioning SQLite, team mode, and
/// Postgres (the required backend). A warning-and-continue would be a
/// failure of this test — RFC 020 requires startup *refusal*, so a wrong
/// configuration never reaches traffic.
///
/// This test deliberately does NOT use `LiveHarness`: LiveHarness scrapes a
/// "cairn-app listening on ..." startup banner off stderr, which a correct
/// implementation of this contract will never print. We spawn the binary
/// directly and assert on its exit status and diagnostic output.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_mode_refuses_sqlite() {
    // Unique path so parallel test runs don't collide on the SQLite file
    // the subprocess is supposed to refuse to open.
    let mut sqlite_path = std::env::temp_dir();
    sqlite_path.push(format!(
        "cairn-rfc020-team-refuse-{}.db",
        uuid::Uuid::new_v4().simple()
    ));
    // Best-effort cleanup even if the subprocess somehow touched it.
    // Must be bound so its Drop runs at the end of the test, not here.
    let _cleanup = RemoveOnDrop {
        path: sqlite_path.clone(),
    };

    let bin = env!("CARGO_BIN_EXE_cairn-app");
    let mut cmd = Command::new(bin);
    cmd.arg("--mode")
        .arg("team")
        .arg("--port")
        .arg("0")
        .arg("--addr")
        .arg("127.0.0.1")
        .arg("--db")
        .arg(sqlite_path.to_str().expect("temp path is valid utf-8"))
        // The seed token is irrelevant — we expect the process to die
        // before it ever reads it — but set it so a regression that
        // panics on missing admin token doesn't masquerade as the
        // RFC 020 refusal we're testing for.
        .env("CAIRN_ADMIN_TOKEN", "seed-admin-team-refuse-sqlite")
        .env("RUST_LOG", "warn")
        .env_remove("CAIRN_LOG_DIR")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // The refusal is a `std::process::exit(1)` at CLI parse time, well
    // before any network or DB init. A healthy failure exits in <1s;
    // a 10s ceiling is a generous sanity bound. If this ever times out,
    // the refusal has regressed into a slow/warning path — the timeout
    // message says exactly that so the failure is self-explanatory.
    let output = timeout(Duration::from_secs(10), cmd.output())
        .await
        .expect(
            "cairn-app did not exit within 10s when started in team mode \
             with a SQLite DB — RFC 020 §'Postgres required for team mode' \
             demands a fast, clear refusal (not a slow-start or warning)",
        )
        .expect("failed to spawn cairn-app binary — did cargo build it?");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("stderr={stderr}\nstdout={stdout}");

    assert!(
        !output.status.success(),
        "cairn-app must refuse to start in team mode with a SQLite DB. \
         Got status={:?}, output:\n{combined}",
        output.status,
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "RFC 020 refusal must exit with code 1; got {:?}\n{combined}",
        output.status.code(),
    );

    // Clear error message requirements:
    // (a) must cite SQLite so the operator knows what's wrong,
    // (b) must cite team mode so they know why the rule applies,
    // (c) must cite Postgres so they know what to do instead.
    let lc = combined.to_lowercase();
    assert!(
        lc.contains("sqlite"),
        "error must mention SQLite. Got:\n{combined}"
    );
    assert!(
        lc.contains("team"),
        "error must mention team mode. Got:\n{combined}"
    );
    assert!(
        lc.contains("postgres"),
        "error must point the operator at Postgres as the remedy. Got:\n{combined}"
    );
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Schedule deletion of `path` when the guard drops. We don't depend on the
/// `scopeguard` crate (not a workspace dev-dep) — a tiny local type is
/// enough and keeps the test file self-contained.
struct RemoveOnDrop {
    path: std::path::PathBuf,
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        for suffix in ["-wal", "-shm"] {
            let mut p = self.path.as_os_str().to_os_string();
            p.push(suffix);
            let _ = std::fs::remove_file(std::path::PathBuf::from(p));
        }
    }
}
