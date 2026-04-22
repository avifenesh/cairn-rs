//! RFC 020 Integration Test #9 — Startup order + `/health/ready` contract.
//!
//! Verifies the readiness gate end-to-end against a REAL `cairn-app`
//! subprocess spawned via `LiveHarness`. Per the user rule, only
//! integration tests against a live process count as evidence for
//! durability/recovery claims; unit tests on `ReadinessState` do not.
//!
//! Test-hook decision (reported in PR body): Option (i).
//! We set `CAIRN_TEST_STARTUP_DELAY_MS` on the cairn-app subprocess so
//! the final `readiness.mark_ready()` is deferred, giving the client a
//! window to observe 503-with-progress on `/health/ready` and 503 on a
//! non-health route. The env var is honored only in `debug_assertions`
//! builds (tests always compile in debug); release builds strip the
//! hook entirely.

use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};

/// How long to defer the final readiness flip. 2s is long enough for
/// a test client to make 2-3 HTTP round trips to `/health/ready` +
/// a non-health route, short enough that the test finishes in ~3s.
const STARTUP_DELAY_MS: u64 = 2_000;

/// Upper bound for polling until the server prints its listening banner.
const BANNER_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound for polling `/health/ready` until it flips to 200.
/// Must exceed `STARTUP_DELAY_MS` with healthy margin.
const READY_TIMEOUT: Duration = Duration::from_secs(10);

struct DelayedHarness {
    base_url: String,
    admin_token: String,
    client: reqwest::Client,
    child: Option<Child>,
}

impl DelayedHarness {
    /// Boot a cairn-app subprocess with a deliberate readiness delay.
    ///
    /// Unlike `LiveHarness::setup()`, we do NOT rotate the admin token
    /// here — rotation hits `/v1/admin/rotate-token`, a non-health
    /// route, which the readiness gate 503s during the delay window.
    /// The test exercises the seed token directly; rotation is covered
    /// by other tests.
    async fn setup() -> Self {
        let (valkey_host, valkey_port) = cairn_fabric::test_harness::valkey_endpoint().await;

        let suffix = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
        let seed_admin = format!("seed-admin-{suffix}-padding");

        let bin = env!("CARGO_BIN_EXE_cairn-app");
        let mut cmd = Command::new(bin);
        cmd.arg("--mode")
            .arg("team")
            .arg("--port")
            .arg("0")
            .arg("--addr")
            .arg("127.0.0.1")
            .arg("--db")
            .arg("memory")
            .env("CAIRN_FABRIC_HOST", &valkey_host)
            .env("CAIRN_FABRIC_PORT", valkey_port.to_string())
            .env("CAIRN_FABRIC_LANE", format!("test-{suffix}"))
            .env("CAIRN_FABRIC_WORKER_ID", format!("worker-{suffix}"))
            .env("CAIRN_FABRIC_INSTANCE_ID", format!("instance-{suffix}"))
            .env("CAIRN_ADMIN_TOKEN", &seed_admin)
            .env(
                "CAIRN_FABRIC_WAITPOINT_HMAC_SECRET",
                "00000000000000000000000000000000000000000000000000000000000000aa",
            )
            .env("CAIRN_FABRIC_WAITPOINT_HMAC_KID", "cairn-test-k1")
            // The dev-only hook: defer the final readiness flip so the
            // test can observe the 503-with-progress contract.
            .env("CAIRN_TEST_STARTUP_DELAY_MS", STARTUP_DELAY_MS.to_string())
            .env("RUST_LOG", "warn,cairn_app=info")
            .env_remove("CAIRN_LOG_DIR")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .expect("failed to spawn cairn-app binary — did cargo build it?");

        let stderr = child.stderr.take().expect("piped stderr present");
        let bound_url = timeout(BANNER_TIMEOUT, wait_for_listening(stderr))
            .await
            .expect("cairn-app did not print listening banner within timeout")
            .expect("cairn-app exited before printing listening banner");

        let base_url = bound_url.replace("0.0.0.0", "127.0.0.1");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("reqwest client builds");

        Self {
            base_url,
            admin_token: seed_admin,
            client,
            child: Some(child),
        }
    }
}

impl Drop for DelayedHarness {
    fn drop(&mut self) {
        drop(self.child.take());
    }
}

async fn wait_for_listening(stderr: tokio::process::ChildStderr) -> Option<String> {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(rest) = line.strip_prefix("cairn-app listening on ") {
            let url = rest.trim().to_owned();
            // Drain the rest of stderr so the pipe doesn't backpressure
            // the child. Tests don't need the noise.
            tokio::spawn(async move { while let Ok(Some(_line)) = lines.next_line().await {} });
            return Some(url);
        }
    }
    None
}

/// RFC 020 Integration Test #9.
///
/// Covers:
/// 1. `GET /health` (liveness) returns 200 immediately after the banner,
///    regardless of readiness state.
/// 2. During the readiness delay window:
///    - `GET /health/ready` returns 503 with a JSON progress body whose
///      `status == "recovering"` and whose `branches` field describes
///      per-branch state.
///    - `GET /v1/runs` returns 503 (readiness middleware blocks
///      non-health routes with `{"status":"recovering", ...}`).
/// 3. After the delay, `GET /health/ready` flips to 200 with the same
///    progress JSON shape, all branches `"complete"`.
#[tokio::test]
async fn health_readiness_transitions_from_recovering_to_ready() {
    let h = DelayedHarness::setup().await;

    // ── 1. Liveness is up immediately ─────────────────────────────────────
    let res = h
        .client
        .get(format!("{}/health", h.base_url))
        .send()
        .await
        .expect("GET /health reaches server");
    assert!(
        res.status().is_success(),
        "GET /health (liveness) must return 2xx while recovering; got {}",
        res.status()
    );

    // ── 2. During the delay: /health/ready is 503 with progress body ──────
    // Race the STARTUP_DELAY_MS window. Poll /health/ready; at least
    // one response in this window must be 503 with the recovering body.
    // If we only ever see 200 here, the test-hook env var wasn't honored
    // (e.g. a release-mode stripped build) — the test fails loud.
    let observed_503 = observe_recovering(&h).await;
    assert!(
        observed_503,
        "expected at least one 503 on /health/ready during the \
         {STARTUP_DELAY_MS}ms readiness delay window. Either the \
         test-hook env var was not honored (debug_assertions off?) \
         or the readiness middleware is not blocking non-health routes."
    );

    // ── 3. Eventually /health/ready flips to 200 ──────────────────────────
    let ready_deadline = Instant::now() + READY_TIMEOUT;
    loop {
        let res = h
            .client
            .get(format!("{}/health/ready", h.base_url))
            .send()
            .await
            .expect("GET /health/ready reaches server");
        let status = res.status();
        let body: serde_json::Value = res.json().await.expect("response is JSON");

        if status == reqwest::StatusCode::OK {
            // RFC 020 contract: the 200 body has the same shape as the 503
            // body, with `status = "ready"` and all branches `"complete"`.
            assert_eq!(
                body.get("status").and_then(|v| v.as_str()),
                Some("ready"),
                "ready body.status must be 'ready'; got {body}"
            );
            let branches = body
                .get("branches")
                .expect("ready body has a 'branches' field");
            // Spot-check the branches that RFC 020 names explicitly.
            for branch in [
                "event_log",
                "sandboxes",
                "runs",
                "tool_result_cache",
                "decision_cache",
            ] {
                let state = branches
                    .get(branch)
                    .and_then(|b| b.get("state"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("<missing>");
                assert_eq!(
                    state, "complete",
                    "branch {branch} must be 'complete' once ready; got \
                     {state} in body {body}"
                );
            }
            break;
        }

        if Instant::now() >= ready_deadline {
            panic!(
                "timed out waiting for /health/ready to flip to 200 within \
                 {READY_TIMEOUT:?}. Last status={status}, body={body}"
            );
        }
        sleep(Duration::from_millis(100)).await;
    }

    // ── 4. After ready, a non-health route is no longer 503'd by the gate. ─
    // Use a bearer-auth'd GET the server should actually handle. We don't
    // assert a specific 200 shape here — just that the readiness middleware
    // no longer short-circuits with 503.
    let res = h
        .client
        .get(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/runs reaches server post-ready");
    assert_ne!(
        res.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "after readiness flip, /v1/runs must not be 503 from the readiness gate"
    );
}

/// Poll `/health/ready` and `/v1/runs` throughout the startup-delay
/// window. Returns true if at least one 503 was observed on
/// `/health/ready` with a body that matches the RFC 020 contract
/// (status="recovering", branches field present) AND a 503 with the
/// recovering body was observed on a non-health route.
///
/// The dual-assertion is important: the PR wires two behaviors
/// (handler + middleware) and both must be verifiable from outside.
async fn observe_recovering(h: &DelayedHarness) -> bool {
    let window_end = Instant::now() + Duration::from_millis(STARTUP_DELAY_MS + 500);
    let mut ready_503_with_progress = false;
    let mut non_health_503 = false;

    while Instant::now() < window_end {
        // Check /health/ready
        if !ready_503_with_progress {
            if let Ok(res) = h
                .client
                .get(format!("{}/health/ready", h.base_url))
                .send()
                .await
            {
                if res.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                    if let Ok(body) = res.json::<serde_json::Value>().await {
                        let status = body.get("status").and_then(|v| v.as_str()).unwrap_or("");
                        let has_branches = body.get("branches").is_some();
                        if status == "recovering" && has_branches {
                            ready_503_with_progress = true;
                        }
                    }
                }
            }
        }

        // Check a non-health route. No bearer required: the readiness gate
        // runs BEFORE auth, so an anonymous GET still gets 503 during
        // recovery (instead of 401). This is the contract RFC 020 wants.
        if !non_health_503 {
            if let Ok(res) = h.client.get(format!("{}/v1/runs", h.base_url)).send().await {
                if res.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                    if let Ok(body) = res.json::<serde_json::Value>().await {
                        if body.get("status").and_then(|v| v.as_str()) == Some("recovering") {
                            non_health_503 = true;
                        }
                    }
                }
            }
        }

        if ready_503_with_progress && non_health_503 {
            return true;
        }
        sleep(Duration::from_millis(50)).await;
    }

    // Report which half missed so a future debugger sees the failure mode.
    if !ready_503_with_progress {
        eprintln!(
            "observe_recovering: never saw 503+progress on /health/ready \
             within the delay window"
        );
    }
    if !non_health_503 {
        eprintln!(
            "observe_recovering: never saw 503+recovering on /v1/runs \
             within the delay window"
        );
    }
    ready_503_with_progress && non_health_503
}
