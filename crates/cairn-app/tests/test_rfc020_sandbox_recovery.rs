//! RFC 020 compliance integration tests for sandbox recovery.
//!
//! Ships the four sandbox-recovery integration tests from the RFC 020
//! §"Integration Tests (Compliance Proof)" list that target the sandbox
//! layer rather than the run-level recovery path:
//!
//! * **Test #3** — sandbox reattach (overlay or reflink). Create a run
//!   with a sandbox, SIGKILL cairn-app, restart; `SandboxService::recover_all`
//!   must find the sandbox and reattach.
//! * **Test #3a** — sandbox preserved: allowlist revoked. Revoke the
//!   repo from the project allowlist between crash and restart;
//!   `SandboxAllowlistRevoked` must be emitted and the sandbox transitions
//!   to `Preserved { reason: AllowlistRevoked }`; the run transitions to
//!   `WaitingApproval`.
//! * **Test #3b** — sandbox preserved: base-revision drift (overlay only).
//!   Move the clone HEAD via `RepoCloneCache::refresh()`, crash, restart;
//!   `SandboxBaseRevisionDrift` emitted and the sandbox transitions to
//!   `Preserved { reason: BaseRevisionDrift }`. A reflink sandbox on the
//!   same tenant does NOT emit drift.
//! * **Test #4** — sandbox lost. Manually delete the sandbox directory,
//!   SIGKILL, restart; the run must transition to `failed` with
//!   `reason: sandbox_lost`.
//!
//! Compliance tripwire audit (as of this commit, HEAD=6d2fb44):
//!
//! * `SandboxService::recover_all()` is wired into cairn-app startup at
//!   `crates/cairn-app/src/main.rs:1086` and flips the `4a.sandboxes`
//!   readiness branch. The BaseRevisionDrift preservation arm is
//!   implemented at `crates/cairn-workspace/src/sandbox/service.rs:614`.
//!   Plumbing exists.
//! * What does NOT exist at the cairn-app HTTP surface today:
//!     - A lightweight HTTP path to provision a sandbox without running
//!       a full LLM-driven orchestrate cycle. Sandbox provisioning lives
//!       inside `working_dir_for_run` (helpers.rs:91), called only from
//!       `POST /v1/runs/:id/orchestrate`, which also does real model
//!       generation — not feasible in CI.
//!     - Recovery-time `SandboxAllowlistRevoked` emission. Allowlist
//!       checks happen on access (RFC 016 integration test
//!       `rfc016_allowlist_revoked_detected_on_access_check`), not on
//!       the recovery sweep. The cairn-app recovery path in
//!       `main.rs:1086` does NOT consult the allowlist.
//!     - Any `sandbox_lost` transition. No code path in the workspace
//!       emits `reason: sandbox_lost` when a sandbox directory has gone
//!       missing. `SandboxService::recover_all()` enumerates via
//!       `provider.list()`, so a deleted sandbox dir simply isn't in
//!       the list — the run's state stays untouched.
//!
//! Because three of the four tests (and the reattach path on a real
//! repo-backed sandbox) require infrastructure not yet landed, each is
//! shipped `#[ignore]` with a precise tripwire comment describing what
//! must land for un-ignore. This matches the PR #77 pattern (Test #7)
//! and registers the test in the file tree so the compliance scoreboard
//! counts it as "partially present with un-ignore precondition".
//!
//! A fifth test, `sandbox_recovery_branch_completes_on_clean_boot`, is
//! the only one that runs by default. It proves the recovery plumbing
//! in `main.rs` actually runs `SandboxService::recover_all()` on every
//! boot, via the observable `/health/ready` `4a.sandboxes` branch.
//! That gives the whole file at least one non-ignored assertion, so the
//! next regression that breaks sandbox recovery plumbing fails CI.

mod support;

use serde_json::Value;
use support::live_fabric::LiveHarness;

/// Serialise env-var-driven test-only seed hooks so two parallel tests
/// do not contaminate each other's cairn-app subprocess via inherited
/// environment. Both `CAIRN_TEST_SEED_LOST_SANDBOX` and
/// `CAIRN_TEST_SEED_ALLOWLIST_REVOKED` are read at subprocess startup
/// and `std::env::set_var` is process-global; tests must hold this
/// mutex across `set_var` + harness spawn (which `fork`s the child and
/// therefore inherits the env snapshot at that instant) + `remove_var`.
///
/// Holding the guard across an `.await` is exactly the point — a
/// non-awaiting critical section would release before the subprocess
/// spawn. `#[allow(clippy::await_holding_lock)]` on the call sites keeps
/// the deliberate semantics obvious.
static ENV_SEED_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ── Shared helpers ──────────────────────────────────────────────────────────

/// Fetch `/health/ready` and return the parsed JSON body. The endpoint
/// returns the full `StartupProgress` regardless of 200/503 status.
async fn fetch_readiness(h: &LiveHarness) -> Value {
    let res = h
        .client()
        .get(format!("{}/health/ready", h.base_url))
        .send()
        .await
        .expect("GET /health/ready reached server");
    // Accept both 200 (ready) and 503 (still flipping) — the progress JSON
    // shape is identical. Tests assert on branch content, not status.
    let status = res.status().as_u16();
    assert!(
        status == 200 || status == 503,
        "GET /health/ready unexpected status: {status}"
    );
    res.json().await.expect("readiness body parses as JSON")
}

/// Extract the `branches.sandboxes` sub-object.
fn sandboxes_branch(readiness: &Value) -> &Value {
    readiness
        .get("branches")
        .and_then(|b| b.get("sandboxes"))
        .unwrap_or_else(|| {
            panic!("readiness body missing branches.sandboxes: {readiness:#?}");
        })
}

// ── Non-ignored plumbing proof ──────────────────────────────────────────────

/// Proves that `SandboxService::recover_all()` runs on every boot and
/// flips the `4a.sandboxes` readiness branch to `complete`. This is the
/// minimal integration evidence that the sandbox-recovery pipeline is
/// wired — a regression that silently skipped the sweep would leave the
/// branch `pending` and `/health/ready` 503 forever, so this test fails
/// noisily if that happens.
///
/// On a fresh harness the sandbox count is 0, which is fine — the
/// assertion is about the branch flipping to a terminal state, not about
/// any particular count. `count` sharing across the shared sandbox base
/// dir (`std::env::temp_dir().join("cairn-workspace-sandboxes")`) means
/// other tests may leave sandboxes on disk; we accept any non-negative
/// count.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sandbox_recovery_branch_completes_on_clean_boot() {
    let h = LiveHarness::setup().await;

    // `setup()` already polled `/health/ready` → 200 via the rotate-token
    // flow's readiness dependency. Read the body directly and assert the
    // sandbox branch is `complete`.
    let readiness = fetch_readiness(&h).await;
    let branch = sandboxes_branch(&readiness);
    let state = branch
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("sandboxes branch has no state: {branch:#?}"));
    assert_eq!(
        state, "complete",
        "RFC 020 §'Startup order' requires sandbox recovery to reach \
         `complete` before the gate opens; got state={state}, \
         full branch: {branch:#?}"
    );

    // `count` must be present and parse as a u64 — this is the field
    // that `BranchStatus::complete(n)` writes. A missing `count` when
    // `state == "complete"` is a schema drift: the serializer dropped
    // the `SandboxRecoverySummary` numbers from the JSON payload, which
    // would silently hide recovery volume from operators. Fail loudly so
    // the drift is caught at the next CI run.
    let count_value = branch.get("count").unwrap_or_else(|| {
        panic!(
            "sandboxes branch has state=complete but missing `count` \
             (schema drift — BranchStatus::complete(n) must serialize \
             count). Branch: {branch:#?}"
        );
    });
    assert!(
        count_value.is_u64(),
        "sandboxes branch `count` must serialize as u64; got {count_value:?} \
         in {branch:#?}"
    );

    // On a completed branch the `detail` field is omitted (per
    // `BranchStatus::complete` which leaves `detail = None`, combined
    // with `#[serde(skip_serializing_if = "Option::is_none")]`). A
    // present `detail` on a `complete` branch means recovery degraded
    // its status — operators should see that surface loudly rather
    // than be hidden behind a no-op assertion.
    assert!(
        branch.get("detail").is_none(),
        "sandboxes branch is complete but carries a `detail` — recovery \
         degraded? Branch: {branch:#?}"
    );
}

// ── Test #3: sandbox reattach ───────────────────────────────────────────────

/// RFC 020 Integration Test #3.
///
/// The exact contract: create a run with an overlay sandbox (Linux) or
/// reflink sandbox (macOS), SIGKILL cairn-app, restart; confirm
/// `SandboxService::recover_all()` finds the sandbox, the run resumes.
///
/// `#[ignore]`d as of this commit because cairn-app has no lightweight
/// HTTP path to provision a real sandbox: `working_dir_for_run`
/// (`helpers.rs:91`) is the only sandbox-provisioning entry point
/// reachable from HTTP, and it is called only from
/// `POST /v1/runs/:id/orchestrate`, which also performs full LLM-driven
/// model generation. CI cannot run an LLM turn.
///
/// Un-ignore this test when EITHER of the following lands:
///
///   (a) A dedicated HTTP surface for "provision a sandbox for run X
///       against repo Y" without requiring orchestrate (e.g.
///       `POST /v1/runs/:id/sandbox` wiring to
///       `SandboxService::provision_or_reconnect`). At that point the
///       test body below switches the `probe` call from asserting 404
///       to seeding the sandbox, SIGKILLing, and reading back
///       `/health/ready` showing `4a.sandboxes.count >= 1` after
///       recovery.
///   (b) A test-only subprocess feature flag that drives the internal
///       `SandboxService` surface directly (e.g. `CAIRN_TEST_SEED_SANDBOX`
///       env var read at boot). Reference: the readiness test's
///       `CAIRN_TEST_STARTUP_DELAY_MS` precedent.
///
/// The assertion below asserts the endpoint is NOT present so that the
/// moment (a) lands, this test starts failing here and forces un-ignore.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "no HTTP path to provision a sandbox without a full LLM \
            orchestrate cycle; see tripwire in doc-comment. As-of \
            commit 6d2fb44 sandbox provisioning only happens via \
            POST /v1/runs/:id/orchestrate (LLM-driven)."]
async fn sandbox_reattach_overlay_or_reflink() {
    let mut h = LiveHarness::setup_with_sqlite().await;

    // When the lightweight sandbox-provision endpoint lands, replace this
    // probe with the actual sandbox-seeding call, then the block below
    // becomes the real assertion.
    let probe = h
        .client()
        .post(format!("{}/v1/runs/placeholder-run/sandbox", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&serde_json::json!({
            "project": { "tenant": h.tenant, "workspace": h.workspace, "project": h.project },
            "strategy": "overlay",
        }))
        .send()
        .await
        .expect("probe reaches server");
    let status = probe.status().as_u16();
    assert!(
        matches!(status, 404 | 405),
        "POST /v1/runs/:id/sandbox unexpectedly returned {status} — the \
         endpoint appears to exist. Un-ignore this test and finish \
         wiring sandbox seeding + post-restart reattach assertion."
    );

    // ── Restart ─────────────────────────────────────────────────────────
    h.sigkill_and_restart().await.expect("sigkill + restart");

    // ── Post-restart: sandbox branch reports reconnection ───────────────
    // Once the above probe actually seeds a sandbox, this must assert
    // `readiness.branches.sandboxes.count >= 1` and, ideally, that the
    // events log contains a `SandboxProvisioned` event from boot 1 and a
    // no-new-provisioning / SandboxActivated-via-reconnect from boot 2.
    let readiness = fetch_readiness(&h).await;
    let _ = sandboxes_branch(&readiness); // shape-only probe until un-ignore
}

// ── Test #3a: sandbox preserved: allowlist revoked ──────────────────────────

/// RFC 020 Integration Test #3a.
///
/// The exact contract: create a run with a `SandboxBase::Repo` sandbox;
/// between crash and restart, revoke the repo from the project's
/// allowlist via `DELETE /v1/projects/:project/repos/:owner/:repo`;
/// restart; confirm `SandboxAllowlistRevoked` emitted and sandbox
/// transitions to `Preserved { reason: AllowlistRevoked }`; run
/// transitions to `WaitingApproval`.
///
/// `#[ignore]`d because the cairn-app recovery path at `main.rs:1086`
/// calls `SandboxService::recover_all()`, which iterates
/// `provider.list()` + `provider.reconnect()` but does NOT consult
/// `project_repo_access` to check whether the sandbox's bound repo is
/// still allowlisted. RFC 016 checks the allowlist on access
/// (`rfc016_allowlist_revoked_detected_on_access_check` in
/// `crates/cairn-workspace/tests/rfc016_integration.rs:258`), not on
/// the recovery sweep.
///
/// Un-ignore when:
///
///   (a) `SandboxService::recover_all()` gains a hook that, for each
///       sandbox with `metadata.repo_id = Some(...)`, asks the repo
///       allowlist whether the repo is still allowed under the
///       sandbox's project; if not, emits `SandboxAllowlistRevoked` and
///       transitions to `Preserved { reason: AllowlistRevoked }`.
///   (b) The run's state machine reacts to `SandboxPreserved` with
///       reason `AllowlistRevoked` by transitioning the run to
///       `WaitingApproval` with a synthesized approval (per RFC 020
///       §"Sandbox preserved: AllowlistRevoked" matrix row).
///   (c) A way to seed the sandbox pre-crash — the same precondition
///       that gates Test #3 above.
///
/// Until all three land, this test probes that the `DELETE` endpoint
/// is reachable (so the revoke step will work) but cannot verify the
/// full contract.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)] // See `ENV_SEED_GUARD` doc-comment.
async fn sandbox_preserved_allowlist_revoked() {
    // Same shape as test #4 (`sandbox_lost_transitions_run_to_failed`):
    // seed via `CAIRN_TEST_SEED_ALLOWLIST_REVOKED` before spawning the
    // cairn-app subprocess so the child inherits the env var. The
    // subprocess writes a registry sidecar for a `SandboxBase::Repo`
    // sandbox whose bound repo is NOT on the project allowlist.
    // `SandboxService::recover_all` emits `SandboxAllowlistRevoked`;
    // `RecoveryService::recover_all` synthesises an approval and
    // transitions the run to `WaitingApproval`.
    let run_id = format!("run-sbx-revoke-{}", uuid::Uuid::new_v4().simple());
    let project_id = format!("p_sbxrevoke_{}", uuid::Uuid::new_v4().simple());
    let tenant_id = "t_sbxrevoke";
    let workspace_id = "w_sbxrevoke";
    let repo_id = "octocat/hello";
    let spec = format!("{run_id}:{tenant_id}:{workspace_id}:{project_id}:{repo_id}");
    let _env_guard = ENV_SEED_GUARD.lock().expect("env seed guard poisoned");
    std::env::set_var("CAIRN_TEST_SEED_ALLOWLIST_REVOKED", &spec);

    let mut h = LiveHarness::setup_with_sqlite().await;
    std::env::remove_var("CAIRN_TEST_SEED_ALLOWLIST_REVOKED");
    drop(_env_guard);

    // SIGKILL + restart exercises the "SIGKILL, restart, confirm"
    // phrasing in RFC 020 §"Run recovery matrix". The seeder is
    // idempotent — on boot 2 the run is `WaitingApproval` (not
    // Pending), so the seed events are skipped and the registry
    // entry's `allowlist_revoked_handled` flag blocks re-emission.
    h.sigkill_and_restart().await.expect("sigkill + restart");

    // Poll `GET /v1/runs/:id` until the run surfaces as
    // `waiting_approval`. The projection upsert on boot 1 appends
    // events; the restart replays them, so the read is consistent
    // post-restart.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut last_state: Option<String> = None;
    let mut last_body: Option<Value> = None;
    while std::time::Instant::now() < deadline {
        let res = h
            .client()
            .get(format!("{}/v1/runs/{}", h.base_url, run_id))
            .bearer_auth(&h.admin_token)
            .header("X-Cairn-Tenant", tenant_id)
            .header("X-Cairn-Workspace", workspace_id)
            .send()
            .await
            .expect("GET /v1/runs/:id reaches server");
        if res.status().as_u16() == 200 {
            let body: Value = res.json().await.expect("run body parses as JSON");
            let state = body
                .get("run")
                .and_then(|r| r.get("state"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            if state.as_deref() == Some("waiting_approval") {
                // Additional assertion: the synthesised approval exists
                // under the project and points at this run.
                let approvals_res = h
                    .client()
                    .get(format!(
                        "{}/v1/approvals?tenant_id={tenant_id}&workspace_id={workspace_id}&project_id={project_id}",
                        h.base_url,
                    ))
                    .bearer_auth(&h.admin_token)
                    .send()
                    .await
                    .expect("GET approvals reaches server");
                if approvals_res.status().as_u16() == 200 {
                    let approvals: Value = approvals_res
                        .json()
                        .await
                        .expect("approvals body parses as JSON");
                    let items = approvals
                        .get("items")
                        .and_then(Value::as_array)
                        .or_else(|| approvals.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let matched = items
                        .iter()
                        .any(|a| a.get("run_id").and_then(Value::as_str) == Some(run_id.as_str()));
                    assert!(
                        matched,
                        "expected synthesised approval for run {run_id}; got approvals={items:?}",
                    );
                }
                return;
            }
            last_state = state;
            last_body = Some(body);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!(
        "run {run_id} did not transition to `waiting_approval` within 5s after \
         sandbox_allowlist_revoked recovery. last_state={last_state:?}, body={:?}",
        last_body
    );
}

// ── Test #3b: sandbox preserved: base-revision drift ────────────────────────

/// RFC 020 Integration Test #3b.
///
/// The exact contract: create an overlay run, call
/// `RepoCloneCache::refresh()` to move the clone HEAD; SIGKILL
/// cairn-app; restart; confirm `SandboxBaseRevisionDrift` emitted and
/// sandbox transitions to `Preserved { reason: BaseRevisionDrift }`;
/// a reflink sandbox on the same tenant does NOT emit drift
/// (physically independent).
///
/// `#[ignore]`d because the emission path exists in
/// `workspace/src/sandbox/service.rs:614` but is unreachable from an
/// integration test today:
///
///   * No HTTP surface to seed a real overlay sandbox (same precondition
///     as Test #3).
///   * No HTTP surface to call `RepoCloneCache::refresh()`. The only
///     existing mutation is the internal `ensure_cloned` call from
///     `working_dir_for_run`; there is no admin endpoint that reopens
///     a clone against a new HEAD.
///
/// Un-ignore when:
///
///   (a) The sandbox-provision path from Test #3 lands.
///   (b) An admin endpoint (or test-only hook) exposes
///       `RepoCloneCache::refresh()` so a test can force HEAD to a new
///       commit between boot 1 and boot 2. Reference shape:
///       `POST /v1/projects/:project/repos/:owner/:repo/refresh`.
///
/// When both land, the test seeds an overlay sandbox and a reflink
/// sandbox on the same tenant, refreshes the clone, SIGKILLs, restarts,
/// and asserts:
///
///   1. `sandbox_base_revision_drift` event present for the overlay
///      sandbox with `expected != actual`.
///   2. No `sandbox_base_revision_drift` event for the reflink sandbox
///      (RFC 016 rule: reflink sandboxes are physically independent
///      post-provision).
///   3. Overlay sandbox moved to `Preserved`, reflink to `Ready`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "emission path live at workspace/src/sandbox/service.rs:614 \
            but unreachable without (a) lightweight sandbox-provision \
            HTTP and (b) RepoCloneCache::refresh HTTP surface."]
async fn sandbox_preserved_base_revision_drift_overlay_only() {
    let mut h = LiveHarness::setup_with_sqlite().await;

    // Probe: the refresh endpoint does not exist yet; confirm that so
    // the moment it's added this assertion fails and forces un-ignore.
    let probe = h
        .client()
        .post(format!(
            "{}/v1/projects/{}/repos/org/foo/refresh",
            h.base_url, h.project
        ))
        .bearer_auth(&h.admin_token)
        .header("X-Cairn-Tenant", &h.tenant)
        .header("X-Cairn-Workspace", &h.workspace)
        .header("X-Cairn-Project", &h.project)
        .send()
        .await
        .expect("probe reaches server");
    let status = probe.status().as_u16();
    assert!(
        matches!(status, 404 | 405),
        "POST /v1/projects/:project/repos/:owner/:repo/refresh \
         unexpectedly returned {status} — un-ignore this test and \
         wire the full overlay-vs-reflink drift assertion."
    );

    h.sigkill_and_restart().await.expect("sigkill + restart");

    // When un-ignored, assert the event log diff between boot-1 and
    // boot-2 contains exactly one `sandbox_base_revision_drift` +
    // `sandbox_preserved` pair (for the overlay) and zero drift events
    // for the reflink.
    //
    // `sigkill_and_restart` already polls `/health/ready` to 200 via
    // `poll_readiness_until_ready`, so the readiness snapshot below
    // is guaranteed to reflect the post-recovery state — no sleeps or
    // secondary polling needed. At un-ignore time, this is the hook
    // where event-log diff + branch-count assertions slot in.
    let _ = fetch_readiness(&h).await;
}

// ── Test #4: sandbox lost ───────────────────────────────────────────────────

/// RFC 020 Integration Test #4.
///
/// Proves the full `sandbox_lost` recovery path end to end:
///
///   1. A recovery-registry sidecar is written for a sandbox whose
///      on-disk root deliberately does not exist (simulating "operator
///      deleted the sandbox directory"). Seeded via the test-only
///      `CAIRN_TEST_SEED_LOST_SANDBOX` env var — the same precedent as
///      `CAIRN_TEST_STARTUP_DELAY_MS` — because cairn-app still has no
///      lightweight HTTP sandbox-provision surface (tests #3 / #3a /
///      #3b remain `#[ignore]`d for that reason).
///   2. The same env-var hook appends `SessionCreated`/`RunCreated`/
///      `RunStateChanged(→Running)` to the store so there's a Running
///      run bound to the lost sandbox when recovery runs.
///   3. `SandboxService::recover_all` detects the registry + missing-
///      root mismatch and emits `SandboxLost`, returning the run in
///      `summary.lost_runs`.
///   4. `RecoveryService::recover_all` consumes `sandbox_lost_runs`
///      and emits `RunStateChanged(Running → Failed,
///      ExecutionError)` + `RecoveryAttempted{reason:"sandbox_lost"}`
///      + `RecoveryCompleted{recovered:false}`.
///   5. `GET /v1/runs/:id` returns `state: "failed"`.
///
/// The test performs one sigkill+restart to exercise the
/// "SIGKILL, restart, confirm" phrasing in the RFC 020 spec row. The
/// seeder is idempotent — on the second boot the run is already
/// terminal and the seed is a no-op, so recovery re-runs cleanly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)] // See `ENV_SEED_GUARD` doc-comment.
async fn sandbox_lost_transitions_run_to_failed() {
    // Seed the env var BEFORE spawning the subprocess. The run_id is
    // harness-scoped so concurrent test runs don't collide in the
    // shared `cairn-workspace-sandboxes` base dir.
    let run_id = format!("run-sbx-lost-{}", uuid::Uuid::new_v4().simple());
    let spec = format!(
        "{run_id}:t_sbxlost:w_sbxlost:p_sbxlost_{}",
        uuid::Uuid::new_v4().simple()
    );
    // Safety: tests share a process, but the env var is only read at
    // cairn-app subprocess startup. `set_var` before `setup_with_sqlite`
    // guarantees the child inherits it. Hold `ENV_SEED_GUARD` across
    // the set/spawn/remove window so the parallel `sandbox_preserved_
    // allowlist_revoked` test does not race with this one via its own
    // `CAIRN_TEST_SEED_*` env var.
    let _env_guard = ENV_SEED_GUARD.lock().expect("env seed guard poisoned");
    std::env::set_var("CAIRN_TEST_SEED_LOST_SANDBOX", &spec);

    let mut h = LiveHarness::setup_with_sqlite().await;
    // Clean up the env var so later tests in the same process don't
    // inherit the seed spec into their own subprocess spawns.
    std::env::remove_var("CAIRN_TEST_SEED_LOST_SANDBOX");
    drop(_env_guard);

    // Boot 1 ran recovery and (on success) transitioned the seeded
    // Running run to Failed. The SIGKILL+restart below proves the
    // transition survives a restart — the run must still read as
    // Failed after the process cycles.
    h.sigkill_and_restart().await.expect("sigkill + restart");

    // Poll `GET /v1/runs/:id` until the run surfaces as `failed`.
    // The projection upsert on boot 1 appends events; the restart
    // replays them, so the read must be consistent post-restart.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut last_state: Option<String> = None;
    let mut last_body: Option<Value> = None;
    while std::time::Instant::now() < deadline {
        let res = h
            .client()
            .get(format!("{}/v1/runs/{}", h.base_url, run_id))
            .bearer_auth(&h.admin_token)
            .header("X-Cairn-Tenant", "t_sbxlost")
            .header("X-Cairn-Workspace", "w_sbxlost")
            .send()
            .await
            .expect("GET /v1/runs/:id reaches server");
        if res.status().as_u16() == 200 {
            let body: Value = res.json().await.expect("run body parses as JSON");
            let state = body
                .get("run")
                .and_then(|r| r.get("state"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            if state.as_deref() == Some("failed") {
                return;
            }
            last_state = state;
            last_body = Some(body);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!(
        "run {run_id} did not transition to `failed` within 5s after \
         sandbox_lost recovery. last_state={last_state:?}, body={:?}",
        last_body
    );
}
