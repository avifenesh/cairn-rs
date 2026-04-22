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

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

/// Bounded wait for the startup banner. 30 s is generous — local dev
/// boots in <2 s, CI cold-starts around 10 s.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Storage backend for a harness subprocess. InMemory is the fast default
/// used by the vast majority of tests; SQLite is opt-in for tests that
/// need DB state to survive a subprocess restart (e.g. the sigkill+restart
/// meta-test).
#[derive(Clone)]
enum HarnessStorage {
    InMemory,
    Sqlite(PathBuf),
}

impl HarnessStorage {
    fn db_arg(&self) -> String {
        match self {
            HarnessStorage::InMemory => "memory".to_owned(),
            // `?mode=rwc` = read+write+create: sqlx won't create the file
            // by default, so we need this for first-boot. On restart the
            // file already exists and rwc still works (no truncation).
            HarnessStorage::Sqlite(path) => format!("sqlite:{}?mode=rwc", path.display()),
        }
    }
}

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
    // Fields captured at setup() so restart() can re-spawn with the same
    // config without re-deriving any of it.
    port: u16,
    suffix: String,
    valkey_host: String,
    valkey_port: u16,
    storage: HarnessStorage,
}

impl LiveHarness {
    pub async fn setup() -> Self {
        Self::setup_with_storage(HarnessStorage::InMemory).await
    }

    /// Variant that persists the event log to a per-harness SQLite file so
    /// projections survive a subprocess restart. Used by the sigkill+restart
    /// meta-test to prove DB state outlives the subprocess.
    pub async fn setup_with_sqlite() -> Self {
        let suffix_hint = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
        let mut path = std::env::temp_dir();
        path.push(format!("cairn-liveharness-{suffix_hint}.db"));
        Self::setup_with_storage(HarnessStorage::Sqlite(path)).await
    }

    async fn setup_with_storage(storage: HarnessStorage) -> Self {
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

        // 4. Spawn the real binary on port 0 (OS-assigned).
        let child =
            spawn_subprocess_internal(0, &seed_admin, &suffix, &valkey_host, valkey_port, &storage);
        let (child, bound_url) = read_listening_banner(child).await;

        // Team mode forces the listener to bind 0.0.0.0 so tests can't dial
        // that directly on every platform. Rewrite to loopback for client
        // requests — same port, always routable.
        let base_url = bound_url.replace("0.0.0.0", "127.0.0.1");
        let port = parse_port(&base_url);

        // 5. Rotate admin token. Both exercises the real operator flow and
        //    narrows the window in which the seed token exists.
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
            port,
            suffix,
            valkey_host,
            valkey_port,
            storage,
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

    /// Send SIGKILL to the subprocess and wait for exit. The harness's
    /// port/token/scope fields are intentionally NOT cleared — `restart()`
    /// uses them to re-spawn. Panics if the child doesn't exit within 5 s,
    /// since a process that refuses SIGKILL is a bug we want to surface
    /// loudly rather than paper over.
    pub async fn sigkill(&mut self) -> std::io::Result<()> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        child.start_kill()?;
        match timeout(Duration::from_secs(5), child.wait()).await {
            Ok(res) => {
                res?;
                Ok(())
            }
            Err(_) => panic!("cairn-app subprocess did not exit within 5s of SIGKILL"),
        }
    }

    /// Spawn a fresh cairn-app subprocess bound to the SAME port, scope,
    /// Valkey, and SQLite file as the original. Seeds `CAIRN_ADMIN_TOKEN`
    /// with the already-rotated token so `self.admin_token` remains valid
    /// without needing another rotation round-trip.
    ///
    /// The OS may hold the previous listener's port in TIME_WAIT briefly;
    /// we spin on connect for up to 3 s waiting for the new subprocess to
    /// bind, then panic if it still isn't ready.
    pub async fn restart(&mut self) -> std::io::Result<()> {
        let child = spawn_subprocess_internal(
            self.port,
            // The new subprocess starts with the already-rotated token as
            // its admin token. No re-rotation needed.
            &self.admin_token,
            &self.suffix,
            &self.valkey_host,
            self.valkey_port,
            &self.storage,
        );
        let (child, bound_url) = read_listening_banner(child).await;
        let base_url = bound_url.replace("0.0.0.0", "127.0.0.1");
        assert_eq!(
            parse_port(&base_url),
            self.port,
            "restart bound to unexpected port: {base_url} (expected {})",
            self.port,
        );
        self.child = Some(child);

        // Spin on `/health/ready` (not `/health`) so we wait for the full
        // init graph — event-log replay, FF engine startup, etc. — rather
        // than just the HTTP listener.
        if !self
            .poll_readiness_until_ready(Duration::from_secs(3))
            .await
        {
            panic!("restarted cairn-app did not become ready within 3s");
        }
        Ok(())
    }

    /// Convenience: `sigkill()` followed by `restart()`.
    pub async fn sigkill_and_restart(&mut self) -> std::io::Result<()> {
        self.sigkill().await?;
        self.restart().await
    }

    /// Poll `url` every 100 ms until the response status equals
    /// `expected_status` or `timeout_dur` expires. Returns `true` on match.
    pub async fn poll_status_until(
        &self,
        url: &str,
        expected_status: StatusCode,
        timeout_dur: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout_dur;
        while Instant::now() < deadline {
            if let Ok(res) = self.client.get(url).send().await {
                if res.status() == expected_status {
                    return true;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    /// Convenience: poll `/health/ready` expecting 200.
    pub async fn poll_readiness_until_ready(&self, timeout_dur: Duration) -> bool {
        self.poll_status_until(
            &format!("{}/health/ready", self.base_url),
            StatusCode::OK,
            timeout_dur,
        )
        .await
    }
}

impl Drop for LiveHarness {
    fn drop(&mut self) {
        // `kill_on_drop(true)` already handles the SIGKILL; taking the
        // child here prevents the default `Drop` from double-logging.
        drop(self.child.take());
        // Best-effort SQLite temp-file cleanup. Ignore errors — the file
        // may already be gone or the OS may be holding it. Use `OsString`
        // append rather than `path.display()` so non-UTF8 paths (rare on
        // Linux test runners but possible anywhere) still clean up.
        if let HarnessStorage::Sqlite(path) = &self.storage {
            let _ = std::fs::remove_file(path);
            for suffix in ["-wal", "-shm"] {
                let mut sidecar = path.as_os_str().to_os_string();
                sidecar.push(suffix);
                let _ = std::fs::remove_file(std::path::PathBuf::from(sidecar));
            }
        }
    }
}

/// Spawn a cairn-app subprocess with the given port and config. Returns
/// a live `Child` with `stderr` piped — caller must drain stderr via
/// [`read_listening_banner`] to extract the bound URL.
fn spawn_subprocess_internal(
    port: u16,
    admin_token: &str,
    suffix: &str,
    valkey_host: &str,
    valkey_port: u16,
    storage: &HarnessStorage,
) -> Child {
    let bin = env!("CARGO_BIN_EXE_cairn-app");
    let mut cmd = Command::new(bin);
    cmd.arg("--mode")
        .arg("team")
        .arg("--port")
        .arg(port.to_string())
        .arg("--addr")
        .arg("127.0.0.1")
        .arg("--db")
        .arg(storage.db_arg())
        .env("CAIRN_FABRIC_HOST", valkey_host)
        .env("CAIRN_FABRIC_PORT", valkey_port.to_string())
        // Unique FF lane so this test's worker queues don't pick up
        // tasks from sibling tests.
        .env("CAIRN_FABRIC_LANE", format!("test-{suffix}"))
        .env("CAIRN_FABRIC_WORKER_ID", format!("worker-{suffix}"))
        .env("CAIRN_FABRIC_INSTANCE_ID", format!("instance-{suffix}"))
        .env("CAIRN_ADMIN_TOKEN", admin_token)
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

    cmd.spawn()
        .expect("failed to spawn cairn-app binary — did cargo build it?")
}

/// Take `stderr` off the child, scan for the listening banner, return the
/// child (with its stderr now background-drained) plus the bound URL.
async fn read_listening_banner(mut child: Child) -> (Child, String) {
    let stderr = child.stderr.take().expect("piped stderr present");
    let bound_url = timeout(STARTUP_TIMEOUT, wait_for_listening(stderr))
        .await
        .expect("cairn-app did not print listening banner within timeout")
        .expect("cairn-app exited before printing listening banner");
    (child, bound_url)
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

/// Extract the port from a base URL of the form `http://host:port`.
fn parse_port(base_url: &str) -> u16 {
    base_url
        .rsplit(':')
        .next()
        .and_then(|s| s.trim_end_matches('/').parse().ok())
        .unwrap_or_else(|| panic!("could not parse port from base_url: {base_url}"))
}
