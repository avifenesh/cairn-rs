//! Full-binary HTTP harness for integration tests.
//!
//! Each `LiveHarness::setup()`:
//!   1. Ensures a shared Valkey testcontainer is up (via
//!      `cairn_fabric::test_harness`).
//!   2. Spawns the real `cairn-app` binary as a child process on an
//!      ephemeral port, pointing at that Valkey.
//!   3. Scrapes the startup banner off stderr to discover the bound port.
//!   4. Rotates the admin token via `POST /v1/admin/rotate-token` so the
//!      dev token only lives in the env for a moment.
//!   5. Returns a handle with `base_url`, `admin_token`, and a uuid-scoped
//!      `ProjectKey` for per-test isolation.
//!
//! `Drop` kills the subprocess. Tests are isolated from each other by
//! uuid-scoped tenant/workspace/project triples — they share the same
//! Valkey container but route to disjoint FF `{p:N}` hash-tag keyspaces
//! and disjoint cairn-store projections.
//!
//! ```no_run
//! let harness = LiveHarness::setup().await;
//! let res = harness
//!     .client()
//!     .post(format!("{}/v1/sessions", harness.base_url))
//!     .bearer_auth(&harness.admin_token)
//!     .json(&serde_json::json!({ "title": "hello" }))
//!     .send()
//!     .await
//!     .unwrap();
//! assert!(res.status().is_success());
//! ```

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

/// Bounded wait for the startup banner. 30 s is generous — local dev
/// boots in <2 s, CI cold-starts around 10 s.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// One cairn-app subprocess driving a uuid-scoped tenant/workspace/project
/// against the shared Valkey testcontainer.
pub struct LiveHarness {
    pub base_url: String,
    pub admin_token: String,
    pub tenant: String,
    pub workspace: String,
    pub project: String,
    child: Option<Child>,
    client: reqwest::Client,
}

impl LiveHarness {
    pub async fn setup() -> Self {
        // 1. Shared Valkey endpoint (first caller boots the container).
        let (valkey_host, valkey_port) = cairn_fabric::test_harness::valkey_endpoint().await;

        // 2. Per-harness uuid scope. Short `_<8hex>` suffix stays under the
        //    Valkey 40-byte hash-tag soft cap while still giving 2^32
        //    collision resistance — ample for a test suite.
        let suffix = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
        let tenant = format!("t_{suffix}");
        let workspace = format!("w_{suffix}");
        let project = format!("p_{suffix}");

        // 3. Bootstrap admin token, rotated immediately after startup.
        let seed_admin = format!("seed-admin-{suffix}-padding");
        let final_admin = format!("test-admin-{suffix}-{}", uuid::Uuid::new_v4().simple());

        // 4. Spawn the real binary. `CARGO_BIN_EXE_cairn-app` resolves to
        //    the compiled binary in target/; cargo guarantees it exists
        //    when integration tests run.
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
            // Unique FF lane so this test's worker queues don't pick up
            // tasks from sibling tests.
            .env("CAIRN_FABRIC_LANE", format!("test-{suffix}"))
            .env("CAIRN_FABRIC_WORKER_ID", format!("worker-{suffix}"))
            .env("CAIRN_FABRIC_INSTANCE_ID", format!("instance-{suffix}"))
            .env("CAIRN_ADMIN_TOKEN", &seed_admin)
            // Waitpoint HMAC: required by FabricConfig to avoid shipping a
            // runtime that would reject every ff_suspend_execution.
            .env(
                "CAIRN_FABRIC_WAITPOINT_HMAC_SECRET",
                "00000000000000000000000000000000000000000000000000000000000000aa",
            )
            .env("CAIRN_FABRIC_WAITPOINT_HMAC_KID", "cairn-test-k1")
            // Silence noisy tracing so stderr is dominated by structured
            // startup lines; integration tests don't need debug spam.
            .env("RUST_LOG", "warn,cairn_app=info")
            // Avoid inheriting the parent test process's log-dir setting.
            .env_remove("CAIRN_LOG_DIR")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .expect("failed to spawn cairn-app binary — did cargo build it?");

        // 5. Scrape bound port off stderr banner: `cairn-app listening on http://127.0.0.1:<port>`.
        let stderr = child.stderr.take().expect("piped stderr present");
        let bound_url = timeout(STARTUP_TIMEOUT, wait_for_listening(stderr))
            .await
            .expect("cairn-app did not print listening banner within timeout")
            .expect("cairn-app exited before printing listening banner");

        // Team mode forces the listener to bind 0.0.0.0 so tests can't dial
        // that directly on every platform. Rewrite to loopback for client
        // requests — same port, always routable.
        let base_url = bound_url.replace("0.0.0.0", "127.0.0.1");

        // 6. Rotate admin token. This both exercises the real operator
        //    flow and narrows the window in which the seed token exists.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client builds");

        let rotate_res = client
            .post(format!("{base_url}/v1/admin/rotate-token"))
            .bearer_auth(&seed_admin)
            .json(&serde_json::json!({ "new_token": final_admin }))
            .send()
            .await
            .expect("rotate-token request reached server");
        assert!(
            rotate_res.status().is_success(),
            "rotate-token failed: {} {}",
            rotate_res.status(),
            rotate_res.text().await.unwrap_or_default(),
        );

        Self {
            base_url,
            admin_token: final_admin,
            tenant,
            workspace,
            project,
            child: Some(child),
            client,
        }
    }

    /// The shared reqwest client.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Standard headers for a test request: bearer + scope.
    pub fn scope_headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("X-Cairn-Tenant", self.tenant.clone()),
            ("X-Cairn-Workspace", self.workspace.clone()),
            ("X-Cairn-Project", self.project.clone()),
        ]
    }
}

impl Drop for LiveHarness {
    fn drop(&mut self) {
        // `kill_on_drop(true)` already handles the SIGKILL; taking the
        // child here prevents the default `Drop` from double-logging.
        drop(self.child.take());
    }
}

/// Read stderr line-by-line until we see the cairn-app startup banner.
/// Returns `Some(base_url)` on match, `None` if the stream ends without one.
async fn wait_for_listening(stderr: tokio::process::ChildStderr) -> Option<String> {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        // Banner format: `cairn-app listening on http://127.0.0.1:<port>`.
        if let Some(rest) = line.strip_prefix("cairn-app listening on ") {
            let url = rest.trim().to_owned();
            // Drain remaining stderr in the background so the pipe
            // doesn't fill up and backpressure the child. When
            // `CAIRN_TEST_ECHO_SERVER_STDERR` is set, tee each line to
            // test stderr — invaluable when a test triggers server-side
            // behavior you want to see (claim contention, FF rejections).
            let echo = std::env::var("CAIRN_TEST_ECHO_SERVER_STDERR").is_ok();
            tokio::spawn(async move {
                while let Ok(Some(line)) = lines.next_line().await {
                    if echo {
                        eprintln!("[cairn-app] {line}");
                    }
                }
            });
            return Some(url);
        }
    }
    None
}
