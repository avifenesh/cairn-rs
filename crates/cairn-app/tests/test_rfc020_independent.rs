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
/// Un-ignored once both preconditions landed:
///   (a) `POST /v1/decisions/evaluate` — RFC 019 pipeline over HTTP;
///   (b) `DecisionRecorded` event persistence + startup replay so the
///       cache is rebuilt from the event log (RFC 020 §"Decision Cache
///       Survival").
///
/// The assertion pattern mirrors Track 3's tool-idempotency test:
///   1. Seed: call /evaluate with an observational tool_invocation
///      (AlwaysCache + 86400s TTL). First call is a fresh evaluation.
///   2. Pre-restart sanity: an equivalent /evaluate returns cache_hit=true.
///   3. sigkill + restart.
///   4. Post-restart: equivalent /evaluate returns cache_hit=true, and the
///      `source` payload references the ORIGINAL pre-restart decision_id,
///      proving the in-memory cache was rebuilt from the event log.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decision_cache_survives_restart() {
    // Boot with SQLite persistence so the event log survives sigkill.
    let mut harness = LiveHarness::setup_with_sqlite().await;

    let body = serde_json::json!({
        "kind": "tool_invocation",
        "tool_name": "grep_search",
        "effect": "observational",
        "principal": { "type": "system" },
        "tenant_id": "default",
        "workspace_id": "default",
        "project_id": "default",
        "correlation_id": "rfc020_test7_pre"
    });

    // ── Step 1: seed a cacheable decision ─────────────────────────────────
    let pre = harness
        .client()
        .post(format!("{}/v1/decisions/evaluate", harness.base_url))
        .bearer_auth(&harness.admin_token)
        .json(&body)
        .send()
        .await
        .expect("pre-restart evaluate reaches server");
    assert_eq!(
        pre.status().as_u16(),
        200,
        "pre-restart /evaluate must succeed"
    );
    let pre_body: serde_json::Value = pre.json().await.expect("pre-restart JSON");
    let original_decision_id = pre_body
        .get("decision_id")
        .and_then(|v| v.as_str())
        .expect("pre-restart decision_id present")
        .to_owned();
    assert_eq!(
        pre_body.get("cached").and_then(|v| v.as_bool()),
        Some(true),
        "observational tool_invocation must be cached post-step-7: {pre_body}"
    );

    // ── Step 2: pre-restart cache hit sanity (in-memory cache) ────────────
    let hit_before = harness
        .client()
        .post(format!("{}/v1/decisions/evaluate", harness.base_url))
        .bearer_auth(&harness.admin_token)
        .json(&serde_json::json!({
            "kind": "tool_invocation",
            "tool_name": "grep_search",
            "effect": "observational",
            "principal": { "type": "system" },
            "correlation_id": "rfc020_test7_pre_hit"
        }))
        .send()
        .await
        .expect("pre-restart second evaluate reaches server");
    let hit_before_body: serde_json::Value = hit_before.json().await.expect("pre-restart hit JSON");
    assert_eq!(
        hit_before_body.get("cache_hit").and_then(|v| v.as_bool()),
        Some(true),
        "second equivalent evaluate must be a cache hit pre-restart: {hit_before_body}"
    );

    // ── Step 3: sigkill + restart ─────────────────────────────────────────
    harness
        .sigkill_and_restart()
        .await
        .expect("sigkill + restart succeeds");

    // ── Step 4: post-restart cache hit proves event-log replay ────────────
    let post = harness
        .client()
        .post(format!("{}/v1/decisions/evaluate", harness.base_url))
        .bearer_auth(&harness.admin_token)
        .json(&serde_json::json!({
            "kind": "tool_invocation",
            "tool_name": "grep_search",
            "effect": "observational",
            "principal": { "type": "system" },
            "correlation_id": "rfc020_test7_post"
        }))
        .send()
        .await
        .expect("post-restart evaluate reaches server");
    assert_eq!(
        post.status().as_u16(),
        200,
        "post-restart /evaluate must succeed"
    );
    let post_body: serde_json::Value = post.json().await.expect("post-restart JSON");
    assert_eq!(
        post_body.get("cache_hit").and_then(|v| v.as_bool()),
        Some(true),
        "post-restart equivalent evaluate must be a cache hit (event-log \
         replay rebuilt the cache): {post_body}"
    );
    // The `original_decision_id` must point at the pre-restart decision —
    // not a new one we just wrote. That's the survival proof: the cache
    // entry is the SAME entry, not a new one that happens to match.
    let source_orig = post_body
        .get("original_decision_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(
        source_orig, original_decision_id,
        "post-restart cache hit must reference the pre-restart decision_id \
         ({original_decision_id}), got body={post_body}"
    );
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

// ── Test #12 (env var path) ──────────────────────────────────────────────────

/// Companion to `team_mode_refuses_sqlite` covering the `DATABASE_URL`
/// environment variable path. Per gemini-code-assist's high-priority
/// finding on PR #77, the original refusal only covered `--db` (CLI
/// parse time); a user specifying a SQLite DB via `DATABASE_URL`
/// would bypass the invariant entirely. The refusal is now enforced
/// after `resolve_storage_from_env`, so both paths are covered. This
/// test pins that behaviour.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_mode_refuses_sqlite_via_database_url_env() {
    let mut sqlite_path = std::env::temp_dir();
    sqlite_path.push(format!(
        "cairn-rfc020-team-refuse-env-{}.db",
        uuid::Uuid::new_v4().simple()
    ));
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
        // No `--db` flag — force the env-var resolution path.
        .env(
            "DATABASE_URL",
            sqlite_path.to_str().expect("temp path is valid utf-8"),
        )
        .env("CAIRN_ADMIN_TOKEN", "seed-admin-team-refuse-sqlite-env")
        .env("RUST_LOG", "warn")
        .env_remove("CAIRN_LOG_DIR")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output = timeout(Duration::from_secs(10), cmd.output())
        .await
        .expect(
            "cairn-app did not exit within 10s with a SQLite DATABASE_URL \
             in team mode — the env-var footgun should be refused exactly \
             like --db",
        )
        .expect("failed to spawn cairn-app binary — did cargo build it?");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("stderr={stderr}\nstdout={stdout}");

    assert_eq!(
        output.status.code(),
        Some(1),
        "team mode + SQLite via DATABASE_URL must exit with code 1; got \
         {:?}\n{combined}",
        output.status.code(),
    );

    let lc = combined.to_lowercase();
    assert!(lc.contains("sqlite"), "must cite SQLite; got:\n{combined}");
    assert!(lc.contains("team"), "must cite team mode; got:\n{combined}");
    assert!(
        lc.contains("postgres"),
        "must cite Postgres as the remedy; got:\n{combined}"
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
