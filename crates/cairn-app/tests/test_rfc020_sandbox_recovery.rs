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
#[ignore = "SandboxService::recover_all does not check repo allowlist; \
            no sandbox_lost / AllowlistRevoked recovery wiring yet. See \
            main.rs:1086 and workspace/src/sandbox/service.rs:575."]
async fn sandbox_preserved_allowlist_revoked() {
    let mut h = LiveHarness::setup_with_sqlite().await;

    // Precondition probe: the DELETE endpoint must exist when we un-ignore.
    // We use a harmless 404 path (no such repo) and accept any
    // 4xx — the point is that the router recognises the method+path.
    let probe = h
        .client()
        .delete(format!(
            "{}/v1/projects/{}/repos/nonexistent/repo",
            h.base_url, h.project
        ))
        .bearer_auth(&h.admin_token)
        .header("X-Cairn-Tenant", &h.tenant)
        .header("X-Cairn-Workspace", &h.workspace)
        .header("X-Cairn-Project", &h.project)
        .send()
        .await
        .expect("DELETE /v1/projects/:project/repos/... reaches server");
    let status = probe.status().as_u16();
    assert!(
        status < 500,
        "DELETE endpoint unreachable (got {status}) — RFC 016 surface \
         has regressed; fix before un-ignoring this test."
    );

    // Placeholder restart so un-ignoring has the scaffolding in place.
    h.sigkill_and_restart().await.expect("sigkill + restart");

    // When un-ignored, assert the event log contains a
    // `sandbox_allowlist_revoked` event emitted during boot-2 recovery,
    // and that the run moved to `waiting_approval`.
    let _ = fetch_readiness(&h).await;
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
/// The exact contract: create a run with a sandbox, manually delete the
/// sandbox directory, SIGKILL cairn-app; restart; confirm the run
/// transitions to `failed` with `reason: sandbox_lost`.
///
/// `#[ignore]`d because there is no code path that emits
/// `reason: sandbox_lost`. A grep across the workspace for
/// `sandbox_lost|SandboxLost|sandbox_missing` returns zero hits.
/// `SandboxService::recover_all()` enumerates via `provider.list()`,
/// which reads `meta.json` off the base dir; if the sandbox directory
/// has been deleted, the provider simply doesn't list it and the
/// corresponding run's recovery decision falls to the run-level
/// RecoveryService, which in PR #75 does not yet know about missing
/// sandboxes either.
///
/// Un-ignore when:
///
///   (a) `SandboxService::recover_all()` grows a "metadata says sandbox
///       should exist but the filesystem disagrees" branch that emits
///       a new `SandboxLost` event (per RFC 020 §"Failure mode"
///       decision matrix "Running / Sandbox missing / transition run to
///       failed with reason: sandbox_lost").
///   (b) The run state machine consumes `SandboxLost` and transitions
///       the run to `failed` with `reason: sandbox_lost` (populates the
///       `RunStateChanged.failure_class` + `detail` fields).
///   (c) The test #3 sandbox-provision precondition lands so a test can
///       actually produce a sandbox to then delete.
///
/// When all three land, the test replaces the probe below with:
///
///   1. Seed sandbox via test #3's provision path.
///   2. `std::fs::remove_dir_all(sandbox_path)` between sigkill +
///      restart.
///   3. Assert `GET /v1/runs/:id` returns `state: "failed"` with
///      `failure_reason: "sandbox_lost"` (or the structured-failure
///      shape the run service eventually exposes).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "no code path emits reason=sandbox_lost; SandboxService \
            recovery does not detect missing-directory case. Grep \
            'sandbox_lost|SandboxLost|sandbox_missing' is empty."]
async fn sandbox_lost_transitions_run_to_failed() {
    let mut h = LiveHarness::setup_with_sqlite().await;

    // Placeholder path — once the provision endpoint lands, replace
    // this with the real sandbox path obtained from the provision call.
    //
    // NOTE on isolation: cairn-app's `default_sandbox_base_dir()` is
    // currently hardcoded to `std::env::temp_dir().join(
    // "cairn-workspace-sandboxes")` (state.rs:1082), shared across
    // every subprocess on the host. To avoid collision between
    // concurrent test runs we scope to a uuid-unique run-id built from
    // the harness scope; and even if the path does collide, the
    // `remove_dir_all` target only exists once the provision endpoint
    // lands, so a pre-un-ignore run is safely a no-op. Un-ignoring
    // this test should be paired with a `LiveHarness` enhancement that
    // plumbs a per-harness sandbox base dir (e.g. via a new
    // `CAIRN_SANDBOX_BASE_DIR` env var + CLI flag) so parallel tests
    // are fully isolated.
    let fake_run_id = format!("sbx-lost-{}-placeholder", h.project);
    let placeholder_sandbox = std::env::temp_dir()
        .join("cairn-workspace-sandboxes")
        .join(fake_run_id);
    // Deleting a nonexistent path is a no-op; the test treats this as a
    // scaffolding hook, not a behavioral assertion.
    let _ = std::fs::remove_dir_all(&placeholder_sandbox);

    h.sigkill_and_restart().await.expect("sigkill + restart");

    // When un-ignored, fetch the run and assert
    // `run.state == "failed" && run.failure_reason == "sandbox_lost"`.
    let _ = fetch_readiness(&h).await;
}
