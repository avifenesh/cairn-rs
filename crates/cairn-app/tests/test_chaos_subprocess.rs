//! Chaos tests against a real `cairn-app` subprocess.
//!
//! Step 2 of 3 in the reality-check suite (provider → chaos → soak).
//! RFC 020 Track 3 proves cairn recovers from SIGKILL; production
//! holds more failure modes than SIGKILL alone. This file closes the
//! cheap, deterministic subset of that gap:
//!
//!   A. SIGSTOP/SIGCONT mid-run — paused subprocess resumes cleanly
//!      and does not report itself ready while frozen.
//!   B. Injected `EventLog::append` failure — the HTTP surface
//!      returns 5xx and the event log stays coherent (no torn
//!      write, no panic, subsequent appends still work).
//!   C. Many rapid SIGKILL+restart cycles — no boot-time state leak,
//!      readiness flips to 200 each cycle, projections replay.
//!
//! None of these run under the default `cargo test` nightly budget —
//! each scenario spawns a real subprocess and the SIGSTOP case holds
//! real wall-clock. Gate locally or run manually:
//!
//! ```bash
//! CAIRN_TEST_VALKEY_URL=redis://localhost:6379 \
//!     cargo test -p cairn-app --test test_chaos_subprocess -- --nocapture
//! ```

#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde_json::json;
use support::live_fabric::LiveHarness;

// ── Scenario A: SIGSTOP + SIGCONT mid-session-create ─────────────────────

/// Deliver a POSIX signal to a child PID via `libc::kill`. Panics on
/// nonzero return — a missing subprocess at this point is a test bug,
/// not a recoverable condition. Unsafe block is minimal and audited:
/// `libc::kill(pid, sig)` has no memory-safety preconditions beyond
/// "pid is an integer".
fn send_signal(pid: u32, signal: libc::c_int) {
    // SAFETY: FFI call with plain integer arguments, no pointers,
    // no unwinding. Failure is reported through the integer return.
    let rc = unsafe { libc::kill(pid as libc::pid_t, signal) };
    assert_eq!(
        rc,
        0,
        "libc::kill(pid={pid}, sig={signal}) failed: errno={}",
        std::io::Error::last_os_error(),
    );
}

/// Send SIGSTOP, verify /health/ready becomes unreachable-ish within a
/// bounded window, send SIGCONT, verify /health/ready flips back to 200.
/// Asserts total elapsed wall-clock stays under a generous budget — if
/// the subprocess hangs after SIGCONT we want a test failure, not a
/// test timeout masquerading as a flake.
#[tokio::test]
async fn sigstop_sigcont_resumes_cleanly() {
    let h = LiveHarness::setup().await;
    let pid = h.subprocess_pid().expect("child PID available pre-SIGSTOP");

    // Sanity: the subprocess is ready before we freeze it.
    assert!(
        h.poll_readiness_until_ready(Duration::from_secs(5)).await,
        "subprocess not ready pre-SIGSTOP",
    );

    let started = Instant::now();

    // 1. Freeze the subprocess. From this moment, no handler runs, no
    //    tokio tick fires, no TCP accept completes on its end.
    send_signal(pid, libc::SIGSTOP);

    // 2. Probe /health/ready with a short per-request timeout. A
    //    frozen subprocess cannot respond 200, so we expect every
    //    probe during the window to either time out or yield a
    //    non-200 (kernel-level TCP RST is still possible if the
    //    listener was already accepted but the task is frozen).
    //
    //    We don't assert "zero 200s" — the request-in-flight when
    //    SIGSTOP lands may have already produced its response
    //    bytes, depending on timing. What we assert is that the
    //    subprocess does NOT wake on its own: we'll verify that
    //    below by measuring readiness BEFORE SIGCONT.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let probe = h
        .client()
        .get(format!("{}/health/ready", h.base_url))
        .timeout(Duration::from_millis(500))
        .send()
        .await;
    match probe {
        Err(_) => {} // expected: timeout / connection stalled
        Ok(res) => {
            // If we somehow got a response, it must not be 200 —
            // a frozen process should not be advertising readiness.
            // (A lingering pre-SIGSTOP response in the socket buffer
            // is possible only if /health/ready was in flight when
            // the signal landed; we didn't issue one, so a 200 here
            // indicates the subprocess is actually running.)
            assert_ne!(
                res.status(),
                StatusCode::OK,
                "frozen subprocess returned 200 /health/ready",
            );
        }
    }

    // 3. Resume. The subprocess should flip back to ready within a
    //    small window — if it doesn't, signal handling or the
    //    async runtime is broken in a way we need to know about.
    send_signal(pid, libc::SIGCONT);
    assert!(
        h.poll_readiness_until_ready(Duration::from_secs(10)).await,
        "subprocess did not recover readiness within 10s of SIGCONT",
    );

    // 4. Durable state is still queryable (no corruption observable
    //    through the API). Create a session; if the subprocess were
    //    wedged post-SIGCONT, this POST would hang or 5xx.
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
        .expect("post-SIGCONT session create reaches subprocess");
    assert_eq!(
        res.status().as_u16(),
        201,
        "post-SIGCONT session create non-201: {}",
        res.text().await.unwrap_or_default(),
    );

    // 5. Hard elapsed-budget cap. SIGSTOP held 3s + ≤10s readiness
    //    recovery + one HTTP round-trip — any more than 30s total
    //    means something is hung and we want the test to fail loudly
    //    rather than masquerade as a flaky slow test.
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(30),
        "scenario A elapsed {elapsed:?} exceeds 30s budget",
    );
}

// ── Scenario B: Injected EventLog::append failure ────────────────────────

/// Boot the subprocess normally, then deliver SIGUSR1 to arm the
/// `InMemoryStore::append` failure hook (`skip=0, fail=1`). Drive one
/// mutation through the HTTP surface and verify:
///
///   1. The HTTP response is 5xx (propagates the error, does not panic).
///   2. The subprocess is still alive and `/health/ready` responds 200.
///   3. A subsequent mutation succeeds — the failure was transient and
///      did not corrupt the event log.
///
/// Arming AFTER startup (via signal, not env var) guarantees the
/// failure budget isn't consumed by bootstrap appends (tenant seed,
/// projection init) — which would otherwise either panic the subprocess
/// during init or leave the token already spent when the test request
/// lands.
#[tokio::test]
async fn fail_next_append_surfaces_cleanly() {
    let h = LiveHarness::setup().await;

    assert!(
        h.poll_readiness_until_ready(Duration::from_secs(10)).await,
        "subprocess not ready pre-arming",
    );

    // Arm one injected failure on the next append. The signal handler
    // is installed just before `axum::serve`, so readiness=200 implies
    // the handler is live.
    let pid = h.subprocess_pid().expect("child PID available pre-SIGUSR1");
    send_signal(pid, libc::SIGUSR1);

    // The handler runs on the tokio runtime; give it a tight bounded
    // window to call `arm_fail_next_append` before the HTTP mutation
    // fires. 100ms is generous.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Drive a PUT to an append-backed endpoint. The first PUT MUST
    // 5xx (hook fires) and the second MUST 2xx (hook drained).
    let retry_url = format!(
        "{}/v1/providers/connections/conn_chaos_b/retry-policy",
        h.base_url,
    );
    let retry_body = json!({
        "max_attempts": 3,
        "backoff_ms": 100,
        "retryable_error_classes": ["transient"],
    });

    let do_put = |attempt: &'static str| {
        let retry_url = retry_url.clone();
        let retry_body = retry_body.clone();
        let headers = h.scope_headers();
        let client = h.client().clone();
        let token = h.admin_token.clone();
        async move {
            let mut req = client.put(&retry_url).bearer_auth(token).json(&retry_body);
            for (k, v) in headers {
                req = req.header(k, v);
            }
            req.send()
                .await
                .unwrap_or_else(|e| panic!("{attempt} retry-policy PUT: {e}"))
        }
    };

    let first = do_put("first").await;
    let first_status = first.status();
    let first_body = first.text().await.unwrap_or_default();
    assert!(
        first_status.is_server_error(),
        "first PUT after SIGUSR1 arm MUST 5xx (got {first_status}: {first_body})",
    );

    // Anti-corruption: subprocess still ready, next PUT succeeds,
    // log not wedged.
    assert!(
        h.poll_readiness_until_ready(Duration::from_secs(5)).await,
        "subprocess not ready after injected append failure",
    );
    let recovery = do_put("recovery").await;
    let recovery_status = recovery.status();
    let recovery_body = recovery.text().await.unwrap_or_default();
    assert!(
        recovery_status.is_success(),
        "post-failure recovery PUT MUST succeed (got {recovery_status}: {recovery_body})",
    );
}

// ── Scenario C': Many rapid SIGKILL+restart cycles ───────────────────────

/// Cycle count. Ten is enough to surface boot-time state leaks (each
/// cycle holds the SQLite WAL sidecar, rotates the in-memory projection
/// state, replays the event log) while staying inside a reasonable
/// local test budget (~30s on a cold machine).
const RAPID_RESTART_CYCLES: usize = 10;

/// SQLite-backed harness so each restart replays a growing event log.
/// Between cycles we append one session, so by the final cycle the
/// replay is exercising RAPID_RESTART_CYCLES events of state.
#[tokio::test]
async fn rapid_restart_cycles_preserve_state() {
    let mut h = LiveHarness::setup_with_sqlite().await;

    let started = Instant::now();
    let mut created_ids = Vec::with_capacity(RAPID_RESTART_CYCLES);

    for cycle in 0..RAPID_RESTART_CYCLES {
        // Create a session so each cycle grows the event log. Uses a
        // cycle-indexed id so we can verify ALL prior sessions
        // survive into each subsequent subprocess.
        let session_id = format!("sess_c_{}_{cycle}", &h.project);
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
            .expect("session create reaches subprocess");
        assert_eq!(
            res.status().as_u16(),
            201,
            "cycle {cycle} session create non-201: {}",
            res.text().await.unwrap_or_default(),
        );
        created_ids.push(session_id);

        // Let the SQLite WAL flush before we yank power — same 500ms
        // discipline as the sigkill meta-test.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Kick.
        h.sigkill_and_restart()
            .await
            .unwrap_or_else(|e| panic!("cycle {cycle} sigkill+restart: {e}"));

        // Readiness must return each cycle.
        assert!(
            h.poll_readiness_until_ready(Duration::from_secs(5)).await,
            "cycle {cycle} subprocess not ready within 5s",
        );
    }

    // After N cycles, every prior session must still be visible —
    // proves replay is idempotent across repeated ungraceful boots
    // and no cycle silently drops events.
    let list_url = format!(
        "{}/v1/sessions?tenant_id={}&workspace_id={}&project_id={}",
        h.base_url, h.tenant, h.workspace, h.project,
    );

    // Projection registration may lag /health/ready by a tick (same
    // precedent as test_live_harness_sigkill.rs); poll for up to 5s.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_items: Vec<serde_json::Value> = Vec::new();
    while Instant::now() < deadline {
        let res = h
            .client()
            .get(&list_url)
            .bearer_auth(&h.admin_token)
            .send()
            .await
            .expect("final list reaches subprocess");
        assert_eq!(res.status().as_u16(), 200, "final list non-200");
        let body: serde_json::Value = res.json().await.expect("final list json");
        last_items = body
            .as_array()
            .cloned()
            .or_else(|| body.get("items").and_then(|v| v.as_array()).cloned())
            .unwrap_or_default();
        if created_ids.iter().all(|want| {
            last_items
                .iter()
                .any(|s| s.get("session_id").and_then(|v| v.as_str()) == Some(want.as_str()))
        }) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    for want in &created_ids {
        assert!(
            last_items
                .iter()
                .any(|s| s.get("session_id").and_then(|v| v.as_str()) == Some(want.as_str())),
            "session {want} lost after {RAPID_RESTART_CYCLES} restart cycles; visible={last_items:?}",
        );
    }

    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(120),
        "{RAPID_RESTART_CYCLES} restart cycles took {elapsed:?} (budget 120s)",
    );
}
